/**
 * esp_mqtt.ino
 * ESP32 Sensor Node — Ultrasonic + PIR over MQTT
 *
 * State machine: CONNECTING_WIFI → CONNECTING_MQTT → RUNNING
 *                                  ↑________________________|
 *                                  (on disconnect / WiFi loss)
 */

#include <ArduinoJson.h>
#include <MQTTClient.h>
#include <WiFi.h>

// ─── Pin Definitions ─────────────────────────────────────────────────────────
#define PIN_LED 2
#define PIN_TRIG 13
#define PIN_ECHO 12
#define PIN_PIRS 14

// ─── Timing Constants (ms)
// ────────────────────────────────────────────────────
#define SONIC_DELAY 400      // interval between ultrasonic readings
#define PUBLISH_INTERVAL 200 // interval between MQTT publishes
#define RECONNECT_DELAY 3000 // wait before attempting reconnect
#define MOTION_HOLD_MS                                                         \
  5000 // keep ultrasonic active for this long after last PIR trigger
#define PIR_DEBOUNCE_COUNT                                                     \
  2 // consecutive HIGH reads required to confirm motion

// ─── LED Blink Periods (ms) ──────────────────────────────────────────────────
#define LED_BLINK_WIFI 300 // slow blink — searching for WiFi
#define LED_BLINK_MQTT 100 // fast blink — connecting to MQTT
#define LED_BLINK_TX 30    // very short pulse on each publish
#define LED_SOLID_ON 0     // 0 = solid (no blinking)

// ─── Network Configuration ───────────────────────────────────────────────────
#define CLIENT_ID "s-002" // CHANGE AS NEEDED

const char WIFI_SSID[] = "TP-Link_F765";  // CHANGE TO YOUR WIFI SSID
const char WIFI_PASSWORD[] = "nottplink"; // CHANGE TO YOUR WIFI PASSWORD
const char MQTT_BROKER_ADDRESS[] = "192.168.50.100"; // CHANGE TO MQTT BROKER IP
const int MQTT_PORT = 1883;
const char MQTT_USERNAME[] = ""; // CHANGE IF REQUIRED
const char MQTT_PASSWORD[] = ""; // CHANGE IF REQUIRED

#define PUBLISH_TOPIC "s-0025/send"
#define SUBSCRIBE_TOPIC "s-0025/receive"

// ─── Device State Machine
// ─────────────────────────────────────────────────────
enum DeviceState {
  STATE_CONNECTING_WIFI,
  STATE_CONNECTING_MQTT,
  STATE_RUNNING,
};
DeviceState deviceState = STATE_CONNECTING_WIFI;

// ─── MQTT & WiFi Objects
// ──────────────────────────────────────────────────────
WiFiClient network;
MQTTClient mqtt(256);

// ─── Sensor Data
// ──────────────────────────────────────────────────────────────
static float distance = 0.0f;
static int pir_res = 0;

// ─── Timing Trackers
// ──────────────────────────────────────────────────────────
unsigned long lastPublishTime = 0;
unsigned long lastSonicRead = 0;
unsigned long lastReconnectAt = 0;
// Initialise far enough in the past that the window is CLOSED at boot.
// Casting avoids the window being open for the first MOTION_HOLD_MS ms.
unsigned long lastMotionTime = (unsigned long)(0UL - MOTION_HOLD_MS - 1UL);

// ─── PIR Debounce ────────────────────────────────────────────────────────────
static uint8_t pirHighCount = 0; // consecutive HIGH samples

// ─── LED State
// ────────────────────────────────────────────────────────────────
unsigned long lastLedToggle = 0;
int ledPhysical = LOW; // actual pin state
int ledBlinkPeriod = LED_BLINK_WIFI;

// ═════════════════════════════════════════════════════════════════════════════
// LED Helpers
// ═════════════════════════════════════════════════════════════════════════════

/** Non-blocking LED blinker — call every loop iteration. */
void updateLed() {
  if (ledBlinkPeriod == LED_SOLID_ON) {
    // Solid ON (used when fully connected)
    if (ledPhysical == LOW) {
      ledPhysical = HIGH;
      digitalWrite(PIN_LED, ledPhysical);
    }
    return;
  }

  if (millis() - lastLedToggle >= (unsigned long)ledBlinkPeriod) {
    lastLedToggle = millis();
    ledPhysical = !ledPhysical;
    digitalWrite(PIN_LED, ledPhysical);
  }
}

/** Blink the LED once synchronously to signal a TX event. */
void ledTxPulse() {
  digitalWrite(PIN_LED, HIGH);
  delay(LED_BLINK_TX);
  digitalWrite(PIN_LED, LOW);
}

// ═════════════════════════════════════════════════════════════════════════════
// Sensor Reading
// ═════════════════════════════════════════════════════════════════════════════

void usonic_distance() {
  digitalWrite(PIN_TRIG, LOW);
  delayMicroseconds(2);
  digitalWrite(PIN_TRIG, HIGH);
  delayMicroseconds(10);
  digitalWrite(PIN_TRIG, LOW);
  long duration = pulseIn(PIN_ECHO, HIGH, 30000UL); // 30ms timeout
  if (duration > 0)
    distance = duration * 0.0172f; // (duration * 344m/s) / 2 / 10 → cm
}

void pir_read() { pir_res = digitalRead(PIN_PIRS); }

// ═════════════════════════════════════════════════════════════════════════════
// WiFi
// ═════════════════════════════════════════════════════════════════════════════

void startWiFi() {
  WiFi.mode(WIFI_STA);
  WiFi.begin(WIFI_SSID, WIFI_PASSWORD);
  Serial.print("[WiFi] Connecting to ");
  Serial.println(WIFI_SSID);
}

bool isWiFiConnected() { return WiFi.status() == WL_CONNECTED; }

// ═════════════════════════════════════════════════════════════════════════════
// MQTT
// ═════════════════════════════════════════════════════════════════════════════

/** Called by the library when an inbound message arrives. */
void messageHandler(String &topic, String &payload) {
  Serial.println("[MQTT] Message received:");
  Serial.print("       topic:   ");
  Serial.println(topic);
  Serial.print("       payload: ");
  Serial.println(payload);
}

/** Attempt a single MQTT connect call. Returns true if successful. */
bool mqttConnectOnce() {
  return mqtt.connect(CLIENT_ID, MQTT_USERNAME, MQTT_PASSWORD);
}

/** Called once after WiFi is up to register broker + callbacks. */
void mqttBegin() {
  mqtt.begin(MQTT_BROKER_ADDRESS, MQTT_PORT, network);
  mqtt.onMessage(messageHandler);
}

/** Subscribe to the command topic after a successful connect. */
void mqttSubscribe() {
  if (mqtt.subscribe(SUBSCRIBE_TOPIC))
    Serial.print("[MQTT] Subscribed to: ");
  else
    Serial.print("[MQTT] Failed to subscribe to: ");
  Serial.println(SUBSCRIBE_TOPIC);
}

// ═════════════════════════════════════════════════════════════════════════════
// Disconnect / Reconnect Handler
// ═════════════════════════════════════════════════════════════════════════════

/**
 * handleDisconnect()
 * Called whenever connectivity loss is detected (WiFi drop or MQTT drop).
 * Transitions the state machine back to the appropriate reconnect state and
 * adjusts the LED pattern so the user has visual feedback.
 */
void handleDisconnect() {
  if (!isWiFiConnected()) {
    Serial.println("[WiFi] Connection lost — reconnecting…");
    deviceState = STATE_CONNECTING_WIFI;
    ledBlinkPeriod = LED_BLINK_WIFI;
    WiFi.reconnect();
  } else {
    // WiFi OK but MQTT dropped
    Serial.println("[MQTT] Disconnected — will reconnect…");
    deviceState = STATE_CONNECTING_MQTT;
    ledBlinkPeriod = LED_BLINK_MQTT;
  }
  lastReconnectAt = millis();
}

// ═════════════════════════════════════════════════════════════════════════════
// MQTT Publish
// ═════════════════════════════════════════════════════════════════════════════

void sendToMQTT(float dist, int pir) {
  StaticJsonDocument<200> doc;
  doc["timestamp"] = millis();
  doc["distance"] = dist;
  doc["movement"] = pir;

  char buf[256];
  serializeJson(doc, buf);

  mqtt.publish(PUBLISH_TOPIC, buf);

  Serial.println("[MQTT] Published:");
  Serial.print("       topic:   ");
  Serial.println(PUBLISH_TOPIC);
  Serial.print("       payload: ");
  Serial.println(buf);

  ledTxPulse(); // brief LED flash on every publish
}

// ═════════════════════════════════════════════════════════════════════════════
// setup()
// ═════════════════════════════════════════════════════════════════════════════

void setup() {
  Serial.begin(115200);
  analogSetAttenuation(ADC_11db);

  pinMode(PIN_TRIG, OUTPUT);
  pinMode(PIN_ECHO, INPUT);
  pinMode(PIN_PIRS, INPUT);
  pinMode(PIN_LED, OUTPUT);

  startWiFi();
  // State + LED already initialised to STATE_CONNECTING_WIFI / LED_BLINK_WIFI
}

// ═════════════════════════════════════════════════════════════════════════════
// loop()
// ═════════════════════════════════════════════════════════════════════════════

void loop() {
  updateLed(); // non-blocking LED management

  // ── State Machine ──────────────────────────────────────────────────────────
  switch (deviceState) {

  // ── STATE: Waiting for WiFi ──────────────────────────────────────────────
  case STATE_CONNECTING_WIFI:
    if (isWiFiConnected()) {
      Serial.print("[WiFi] Connected — IP: ");
      Serial.println(WiFi.localIP());
      mqttBegin();
      deviceState = STATE_CONNECTING_MQTT;
      ledBlinkPeriod = LED_BLINK_MQTT;
      lastReconnectAt = millis();
    }
    break;

  // ── STATE: Waiting for MQTT ──────────────────────────────────────────────
  case STATE_CONNECTING_MQTT:
    if (!isWiFiConnected()) {
      handleDisconnect();
      break;
    }
    if (millis() - lastReconnectAt >= RECONNECT_DELAY) {
      Serial.println("[MQTT] Attempting connection…");
      if (mqttConnectOnce()) {
        Serial.println("[MQTT] Connected to broker!");
        mqttSubscribe();
        deviceState = STATE_RUNNING;
        ledBlinkPeriod = LED_SOLID_ON; // solid LED = all good
      } else {
        Serial.println("[MQTT] Connection failed — will retry.");
        lastReconnectAt = millis();
      }
    }
    break;

  // ── STATE: Normal Operation ──────────────────────────────────────────────
  case STATE_RUNNING:
    // Check connectivity first
    if (!isWiFiConnected() || !mqtt.connected()) {
      handleDisconnect();
      break;
    }

    mqtt.loop(); // process incoming MQTT messages

    // PIR debounce — require PIR_DEBOUNCE_COUNT consecutive HIGH reads
    pir_read();
    if (pir_res) {
      if (pirHighCount < PIR_DEBOUNCE_COUNT)
        pirHighCount++;
    } else {
      pirHighCount = 0; // any LOW immediately resets the counter
    }
    bool motionConfirmed = (pirHighCount >= PIR_DEBOUNCE_COUNT);
    if (motionConfirmed) {
      lastMotionTime = millis(); // refresh hold-window only on confirmed motion
    }

    // Ultrasonic — only fire while inside the motion hold-window
    bool inMotionWindow = (millis() - lastMotionTime < MOTION_HOLD_MS);
    if (inMotionWindow) {
      if (millis() - lastSonicRead >= SONIC_DELAY) {
        usonic_distance();
        lastSonicRead = millis();
      }
    } else {
      distance = 0.0f; // hold-window expired — clear stale reading
    }

    // Publish sensor data
    if (millis() - lastPublishTime >= PUBLISH_INTERVAL) {
      sendToMQTT(distance, (int)motionConfirmed);
      lastPublishTime = millis();
    }
    break;
  }

  delay(100);
}
