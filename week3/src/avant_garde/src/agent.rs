use crate::sensor::SensorData;
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

const AGENT_EXPIRATION_TIME: Duration = Duration::from_millis(5000); // 5 seconds of expiration time
const MAX_QUEUE_LENGTH: usize = 128;
const DEFAULT_SANITIZE_DURATION: Duration = Duration::from_millis(4000);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AgentState {
    ACTIVE = 0,
    EXPIRED = 1,
}

#[derive(Clone, Copy, Debug)]
pub struct Coordinate {
    pub x: f32,
    pub y: f32,
}

pub struct AgentInfo {
    pub id: String,
    pub state: AgentState,
    pub position: Option<Coordinate>,
    pub last_access: Instant,
    pub num_access: u32,
}

// struct for each agent record
pub struct AgentRecord {
    last_access: Instant,
    num_access: u32,
    state: AgentState,
    position: Option<Coordinate>,
    data: VecDeque<SensorData>,
}

pub enum AgentRecordError {
    NotExist,
}

pub struct AgentStore {
    store: HashMap<String, AgentRecord>,
    last_sanitize: Instant,
    sanitize_dur: Duration,
}

impl AgentStore {
    // create a new agent store
    pub fn new(sanitize_dur: Option<Duration>) -> Self {
        AgentStore {
            store: HashMap::new(),
            last_sanitize: Instant::now(),
            sanitize_dur: sanitize_dur.unwrap_or(DEFAULT_SANITIZE_DURATION),
        }
    }

    pub fn get_agents(&self) -> Vec<AgentInfo> {
        self.store.iter().map(|(id, record)| AgentInfo {
            id: id.clone(),
            state: record.state,
            position: record.position,
            last_access: record.last_access,
            num_access: record.num_access,
        }).collect()
    }

    // For each agents in the agent store,
    // check if they are expired and change the state based on result
    pub fn update_expiration(&mut self) -> usize {
        let current_time = Instant::now();
        self.store.iter_mut().for_each(|(_, value)| {
            let passed_time = current_time.duration_since(value.last_access);
            if passed_time > AGENT_EXPIRATION_TIME {
                value.state = AgentState::EXPIRED;
            }
        });

        if Instant::now().duration_since(self.last_sanitize) > self.sanitize_dur {
            self.last_sanitize = Instant::now();
            return self.sanitize();
        }

        0
    }

    // Update the store for new agent activity
    // Returns the number of agents in the store after update
    pub fn update_agent(&mut self, agent_id: &str, data: Option<SensorData>) -> usize {
        let current_time = Instant::now();
        if let Some(record) = self.store.get_mut(agent_id) {
            record.last_access = current_time;
            record.num_access += 1;
            record.state = AgentState::ACTIVE;
        } else {
            let record = AgentRecord {
                last_access: current_time,
                num_access: 1,
                position: None,
                state: AgentState::ACTIVE,
                data: VecDeque::with_capacity(MAX_QUEUE_LENGTH),
            };
            self.store.insert(String::from(agent_id), record);
        }

        // if the data sample is also provided then add them into
        // the agent data store
        if let Some(sensor_data) = data {
            let record = self.store.get_mut(agent_id).unwrap();
            record.data.push_back(sensor_data);
            if record.data.len() == record.data.capacity() {
                record.data.pop_front();
            }
        }

        self.store.len()
    }

    // method to set the position of the agent,
    // returns false if the record is not present for the agent
    pub fn set_agent_position(&mut self, agent_id: &str, position: Coordinate) -> bool {
        if let Some(record) = self.store.get_mut(agent_id) {
            record.position = Some(position);
            return true;
        }
        false
    }

    // get the most recent distance reading for an agent (0.0 if none)
    pub fn get_latest_distance(&self, agent_id: &str) -> f32 {
        self.store.get(agent_id)
            .and_then(|r| r.data.back())
            .map(|d| d.distance)
            .unwrap_or(0.0)
    }

    // get the most recent movement flag for an agent
    pub fn get_latest_movement(&self, agent_id: &str) -> u8 {
        self.store.get(agent_id)
            .and_then(|r| r.data.back())
            .map(|d| d.movement)
            .unwrap_or(0)
    }

    // method to get the slice of activation data for a agent
    pub fn get_activation_slice(&self, agent_id: &str) -> Result<Vec<u8>, AgentRecordError> {
        let record = self.store.get(agent_id).ok_or(AgentRecordError::NotExist)?;
        Ok(record.data.iter().map(|row| row.movement).collect())
    }

    // method to get the slice of distance data for a agent
    pub fn get_distance_slice(&self, agent_id: &str) -> Result<Vec<f32>, AgentRecordError> {
        let record = self.store.get(agent_id).ok_or(AgentRecordError::NotExist)?;
        Ok(record.data.iter().map(|row| row.distance).collect())
    }

    // method to get the slice of timestamp data for a agent
    pub fn get_timestamp_slice(&self, agent_id: &str) -> Result<Vec<u64>, AgentRecordError> {
        let record = self.store.get(agent_id).ok_or(AgentRecordError::NotExist)?;
        Ok(record.data.iter().map(|row| row.timestamp).collect())
    }

    // Remove the expired agents from the store
    // Returns the number of agents removed
    fn sanitize(&mut self) -> usize {
        // build a set of ids of expired agents
        let mut expired_agents: HashSet<String> = HashSet::new();
        self.store.iter().for_each(|(key, value)| {
            if value.state == AgentState::EXPIRED {
                let _ = expired_agents.insert(String::from(key));
            };
        });

        let count_expired_agents = expired_agents.len();
        expired_agents.iter().for_each(|agent_id| {
            self.store.remove(agent_id);
        });

        count_expired_agents// return the number of agents removed
                                     // from the store
    }
}
