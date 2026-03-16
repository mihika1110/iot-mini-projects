use serde::Deserialize;
#[derive(Debug, Deserialize)]
pub struct SensorData {
    pub timestamp: u64,
    pub distance: f32,
    pub movement: u8,
}
