use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum JobStatus {
    Pending,
    Processing,
    Completed,
    Failed(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub task: String,
    pub payload: serde_json::Value,
    pub status: JobStatus,
    pub created_at: String,
    pub completed_at: Option<String>,
    pub result: Option<serde_json::Value>,
}

pub struct JobQueue {
    pub jobs: Arc<RwLock<HashMap<String, Job>>>,
    pub queue: Arc<Mutex<VecDeque<String>>>, // Menyimpan ID job yang pending
}

impl JobQueue {
    pub fn new() -> Self {
        Self {
            jobs: Arc::new(RwLock::new(HashMap::new())),
            queue: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    pub async fn submit(&self, task: String, payload: serde_json::Value) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();

        let job = Job {
            id: id.clone(),
            task,
            payload,
            status: JobStatus::Pending,
            created_at: now,
            completed_at: None,
            result: None,
        };

        let task_name = job.task.clone();
        let mut jobs = self.jobs.write().await; 
        jobs.insert(id.clone(), job);
        drop(jobs);
        
        let mut queue = self.queue.lock().await;
        queue.push_back(id.clone());
        
        println!("[Queue] Job submitted: {} (Task: {})", id, task_name);
        id
    }

    pub async fn get_status(&self, id: &str) -> Option<Job> {
        let jobs = self.jobs.read().await;
        jobs.get(id).cloned()
    }

    pub async fn dequeue(&self) -> Option<String> {
        let mut queue = self.queue.lock().await;
        queue.pop_front()
    }

    pub async fn mark_processing(&self, id: &str) {
        let mut jobs = self.jobs.write().await;
        if let Some(job) = jobs.get_mut(id) {
            job.status = JobStatus::Processing;
        }
    }

    pub async fn mark_completed(&self, id: &str, result: serde_json::Value) {
        let mut jobs = self.jobs.write().await;
        if let Some(job) = jobs.get_mut(id) {
            job.status = JobStatus::Completed;
            job.result = Some(result);
            job.completed_at = Some(chrono::Utc::now().to_rfc3339());
            println!("[Queue] Job completed: {}", id);
        }
    }

    #[allow(dead_code)]
    pub async fn mark_failed(&self, id: &str, error: String) {
        let mut jobs = self.jobs.write().await;
        if let Some(job) = jobs.get_mut(id) {
            job.status = JobStatus::Failed(error);
            job.completed_at = Some(chrono::Utc::now().to_rfc3339());
            println!("[Queue] Job failed: {}", id);
        }
    }
}