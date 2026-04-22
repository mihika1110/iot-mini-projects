import cv2
import numpy as np
import pytesseract
import onnxruntime as ort
import re
import csv
import os
from datetime import datetime
from collections import Counter, deque
from picamera2 import Picamera2

# SETTINGS
MODEL_PATH       = "yolov8n-license_plate.onnx"
IMG_SIZE         = 512
CONF_THRESHOLD   = 0.5
NMS_THRESHOLD    = 0.3

VOTE_WINDOW      = 12
MIN_VOTES_NEEDED = 4
MIN_PLATE_LEN    = 6

CSV_FILE         = "data.csv"

pytesseract.pytesseract.tesseract_cmd = "/usr/bin/tesseract"
OCR_CONFIG = '--psm 7 --oem 3 -c tessedit_char_whitelist=ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789'


# CSV SETUP — create file with headers if it doesn't exist
if not os.path.exists(CSV_FILE):
    with open(CSV_FILE, "w", newline="") as f:
        writer = csv.writer(f)
        writer.writerow(["timestamp", "plate", "confidence", "format"])
    print(f"📄 Created {CSV_FILE}")
else:
    print(f"📄 Appending to existing {CSV_FILE}")


def save_to_csv(plate, confidence, fmt):
    timestamp = datetime.now().strftime("%Y-%m-%d %H:%M:%S")
    with open(CSV_FILE, "a", newline="") as f:
        writer = csv.writer(f)
        writer.writerow([timestamp, plate, f"{confidence:.2f}", fmt])


# O / 0 CORRECTION
def fix_O_zero(text):
    t = text.upper()
    bh_loose = re.match(r'^(\d{2})(BH)([0-9O]{4})([A-Z]{1,2})$', t)
    if bh_loose:
        prefix, bh, middle, suffix = bh_loose.groups()
        return prefix + bh + middle.replace('O', '0') + suffix
    std_loose = re.match(r'^([A-Z]{2})([0-9O]{2})([A-Z]{1,3})([0-9O]{4})$', t)
    if std_loose:
        state, dist, series, num = std_loose.groups()
        return state + dist.replace('O', '0') + series + num.replace('O', '0')
    return t


# INDIAN PLATE VALIDATORS
STANDARD_PATTERN = re.compile(r'^[A-Z]{2}\d{2}[A-Z]{1,3}\d{4}$')
BH_PATTERN       = re.compile(r'^\d{2}BH\d{4}[A-Z]{1,2}$')
SHORT_PATTERN    = re.compile(r'^[A-Z0-9]{6,12}$')

def validate_plate(text):
    if len(text) < MIN_PLATE_LEN:
        return text, 'LOW'
    if STANDARD_PATTERN.match(text):
        return text, 'HIGH'
    if BH_PATTERN.match(text):
        return text, 'HIGH'
    if SHORT_PATTERN.match(text):
        return text, 'MED'
    return text, 'LOW'


# TEMPORAL VOTER
class PlateVoter:
    def __init__(self, window=VOTE_WINDOW, min_votes=MIN_VOTES_NEEDED):
        self.window      = window
        self.min_votes   = min_votes
        self._buffers    = {}
        self._stable     = {}
        self._printed    = {}
        self._regions    = {}
        self._next_id    = 0

    @staticmethod
    def _iou(a, b):
        ix1, iy1 = max(a[0], b[0]), max(a[1], b[1])
        ix2, iy2 = min(a[2], b[2]), min(a[3], b[3])
        inter = max(0, ix2 - ix1) * max(0, iy2 - iy1)
        if inter == 0:
            return 0.0
        return inter / ((a[2]-a[0])*(a[3]-a[1]) + (b[2]-b[0])*(b[3]-b[1]) - inter)

    def _get_region_id(self, box):
        best_id, best_iou = None, 0.25
        for rid, rbox in self._regions.items():
            iou = self._iou(box, rbox)
            if iou > best_iou:
                best_id, best_iou = rid, iou
        if best_id is None:
            best_id = self._next_id
            self._next_id += 1
            self._printed[best_id] = set()
        self._regions[best_id] = box
        return best_id

    def push(self, box, text):
        if len(text) < MIN_PLATE_LEN:
            return None, False
        rid = self._get_region_id(box)
        if rid not in self._buffers:
            self._buffers[rid] = deque(maxlen=self.window)
        self._buffers[rid].append(text)
        buf = self._buffers[rid]
        if len(buf) < self.min_votes:
            return None, False
        counts = Counter(buf)
        top_text, top_count = counts.most_common(1)[0]
        if top_count < self.min_votes:
            return None, False
        self._stable[rid] = top_text
        should_print = (top_text not in self._printed[rid])
        if should_print:
            self._printed[rid].add(top_text)
        return top_text, should_print

voter = PlateVoter()

# NMS
def apply_nms(detections, conf_thresh, nms_thresh, orig_w, orig_h):
    boxes, confidences = [], []
    for det in detections.T:
        if len(det) < 5:
            continue
        x, y, bw, bh, conf = det[:5]
        if conf < conf_thresh:
            continue
        x1 = int((x - bw / 2) * orig_w / IMG_SIZE)
        y1 = int((y - bh / 2) * orig_h / IMG_SIZE)
        x2 = int((x + bw / 2) * orig_w / IMG_SIZE)
        y2 = int((y + bh / 2) * orig_h / IMG_SIZE)
        x1, y1 = max(0, x1), max(0, y1)
        x2, y2 = min(orig_w, x2), min(orig_h, y2)
        if x2 - x1 < 50 or y2 - y1 < 20:
            continue
        boxes.append([x1, y1, x2 - x1, y2 - y1])
        confidences.append(float(conf))
    if not boxes:
        return []
    indices = cv2.dnn.NMSBoxes(boxes, confidences, conf_thresh, nms_thresh)
    if len(indices) == 0:
        return []
    results = []
    for i in indices.flatten():
        x, y, bw, bh = boxes[i]
        results.append((x, y, x + bw, y + bh, confidences[i]))
    return results

# DESKEW
def deskew(gray_img):
    edges = cv2.Canny(gray_img, 50, 150, apertureSize=3)
    lines = cv2.HoughLines(edges, 1, np.pi / 180, threshold=60)
    if lines is None:
        return gray_img
    angles = []
    for rho, theta in lines[:, 0]:
        angle = (theta - np.pi / 2) * 180 / np.pi
        if abs(angle) < 20:
            angles.append(angle)
    if not angles:
        return gray_img
    median_angle = np.median(angles)
    if abs(median_angle) < 0.5:
        return gray_img
    h, w = gray_img.shape
    M = cv2.getRotationMatrix2D((w // 2, h // 2), median_angle, 1.0)
    return cv2.warpAffine(gray_img, M, (w, h),
                          flags=cv2.INTER_CUBIC,
                          borderMode=cv2.BORDER_REPLICATE)


# OCR PREPROCESSING
def preprocess_for_ocr(plate_bgr):
    h, w = plate_bgr.shape[:2]
    target_h  = 64
    plate_bgr = cv2.resize(plate_bgr,
                            (max(1, int(w * target_h / h)), target_h),
                            interpolation=cv2.INTER_CUBIC)
    gray = cv2.cvtColor(plate_bgr, cv2.COLOR_BGR2GRAY)
    clahe = cv2.createCLAHE(clipLimit=2.0, tileGridSize=(4, 4))
    gray  = clahe.apply(gray)
    gray  = cv2.bilateralFilter(gray, 9, 75, 75)
    gray  = deskew(gray)
    _, thresh = cv2.threshold(gray, 0, 255,
                               cv2.THRESH_BINARY + cv2.THRESH_OTSU)
    if np.sum(thresh == 0) > thresh.size * 0.5:
        thresh = cv2.bitwise_not(thresh)
    kernel = cv2.getStructuringElement(cv2.MORPH_RECT, (1, 1))
    thresh = cv2.dilate(thresh, kernel, iterations=1)
    return thresh


# DUAL-PASS OCR
def run_ocr(processed_img):
    def ocr_pass(img):
        raw = pytesseract.image_to_string(img, config=OCR_CONFIG)
        return fix_O_zero(''.join(c for c in raw if c.isalnum()))

    text1 = ocr_pass(processed_img)
    big   = cv2.resize(processed_img, None, fx=2, fy=2,
                       interpolation=cv2.INTER_CUBIC)
    text2 = ocr_pass(big)

    priority = {'HIGH': 3, 'MED': 2, 'LOW': 1}
    _, tag1 = validate_plate(text1)
    _, tag2 = validate_plate(text2)
    return text1 if priority[tag1] >= priority[tag2] else text2

# DRAW OVERLAY
def draw_overlay(frame, x1, y1, x2, y2, text, det_conf, vtag, pending=False):
    colour_map = {
        'HIGH':    (0, 255, 0),
        'MED':     (0, 200, 255),
        'LOW':     (0, 0, 255),
        'PENDING': (180, 180, 180),
    }
    colour = colour_map['PENDING'] if pending else colour_map.get(vtag, colour_map['LOW'])
    label  = f"{'...' if pending else text}  ({det_conf:.2f})"
    if not pending:
        label += f" [{vtag}]"
    cv2.rectangle(frame, (x1, y1), (x2, y2), colour, 2)
    (lw, lh), baseline = cv2.getTextSize(label, cv2.FONT_HERSHEY_SIMPLEX, 0.6, 2)
    label_y = max(y1 - 10, lh + 4)
    cv2.rectangle(frame,
                  (x1, label_y - lh - baseline - 2),
                  (x1 + lw, label_y + baseline),
                  colour, cv2.FILLED)
    cv2.putText(frame, label, (x1, label_y - baseline),
                cv2.FONT_HERSHEY_SIMPLEX, 0.6, (0, 0, 0), 2)


# LOAD MODEL
print("Loading ONNX model...")
session    = ort.InferenceSession(MODEL_PATH)
input_name = session.get_inputs()[0].name
print("✅ Model loaded.")

# START CAMERA
print("Starting camera...")
picam2 = Picamera2()
picam2.configure(picam2.create_preview_configuration(main={"size": (640, 480)}))
picam2.start()
print("✅ Camera started. Press 'q' to exit.\n")

# MAIN LOOP
while True:
    frame = picam2.capture_array()
    frame = cv2.cvtColor(frame, cv2.COLOR_RGB2BGR)
    h, w  = frame.shape[:2]

    img = cv2.resize(frame, (IMG_SIZE, IMG_SIZE))
    img = img.astype(np.float32) / 255.0
    img = np.transpose(img, (2, 0, 1))
    img = np.expand_dims(img, axis=0)

    outputs    = session.run(None, {input_name: img})
    detections = outputs[0][0]
    kept_boxes = apply_nms(detections, CONF_THRESHOLD, NMS_THRESHOLD, w, h)

    for (x1, y1, x2, y2, conf) in kept_boxes:
        plate_crop = frame[y1:y2, x1:x2]
        if plate_crop.size == 0:
            continue

        processed = preprocess_for_ocr(plate_crop)
        raw_text  = run_ocr(processed)

        box = (x1, y1, x2, y2)
        stable_text, should_print = voter.push(box, raw_text)

        if stable_text is None:
            draw_overlay(frame, x1, y1, x2, y2, '', conf, 'LOW', pending=True)
        else:
            _, vtag = validate_plate(stable_text)

            if should_print and vtag in ('HIGH', 'MED'):
                print(f"🚗  Plate: {stable_text:<14}  det_conf={conf:.2f}  format={vtag}")
                save_to_csv(stable_text, conf, vtag)   # ← saves to data.csv
                print(f"   💾 Saved to {CSV_FILE}")

            draw_overlay(frame, x1, y1, x2, y2, stable_text, conf, vtag)

    cv2.imshow("License Plate Detection", frame)
    if cv2.waitKey(1) & 0xFF == ord('q'):
        break

# CLEANUP
picam2.stop()
cv2.destroyAllWindows()
print("👋 Stopped.")
