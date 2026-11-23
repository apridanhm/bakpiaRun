use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

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
    pub queue: Arc<Mutex<VecDeque<String>>>,
    finished_order: Arc<Mutex<VecDeque<String>>>,
    max_jobs: AtomicUsize,
}

impl JobQueue {
    pub fn new(max_jobs: usize) -> Self {
        Self {
            jobs: Arc::new(RwLock::new(HashMap::new())),
            queue: Arc::new(Mutex::new(VecDeque::new())),
            finished_order: Arc::new(Mutex::new(VecDeque::new())),
            max_jobs: AtomicUsize::new(max_jobs.max(1)),
        }
    }

    pub fn set_max_jobs(&self, max_jobs: usize) {
        self.max_jobs.store(max_jobs.max(1), Ordering::Release);
    }

    /// Submits a job to the queue while strictly enforcing `max_jobs` capacity
    /// and evicting old completed/failed job records so RAM stays bounded.
    pub async fn submit(
        &self,
        task: String,
        payload: serde_json::Value,
    ) -> Result<String, String> {
        let cap = self.max_jobs.load(Ordering::Acquire);

        {
            let q = self.queue.lock().await;
            if q.len() >= cap {
                return Err(format!(
                    "Queue is full (pending jobs reached max_jobs limit of {})",
                    cap
                ));
            }
        }

        // Evict oldest finished job records if stored history exceeds capacity
        {
            let mut jobs = self.jobs.write().await;
            if jobs.len() >= cap {
                let mut fin = self.finished_order.lock().await;
                while jobs.len() >= cap {
                    if let Some(old_id) = fin.pop_front() {
                        jobs.remove(&old_id);
                    } else {
                        break;
                    }
                }
            }
            if jobs.len() >= cap {
                return Err(format!(
                    "Queue capacity exhausted (active jobs reached max_jobs limit of {})",
                    cap
                ));
            }

            let id = uuid::Uuid::new_v4().to_string();
            let now = chrono::Utc::now().to_rfc3339();
            let task_name = task.clone();

            let job = Job {
                id: id.clone(),
                task,
                payload,
                status: JobStatus::Pending,
                created_at: now,
                completed_at: None,
                result: None,
            };

            jobs.insert(id.clone(), job);
            drop(jobs);

            let mut queue = self.queue.lock().await;
            queue.push_back(id.clone());

            println!("[Queue] Job submitted: {} (Task: {})", id, task_name);
            Ok(id)
        }
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
        {
            let mut jobs = self.jobs.write().await;
            if let Some(job) = jobs.get_mut(id) {
                job.status = JobStatus::Completed;
                job.result = Some(result);
                job.completed_at = Some(chrono::Utc::now().to_rfc3339());
                println!("[Queue] Job completed: {}", id);
            }
        }
        let mut fin = self.finished_order.lock().await;
        fin.push_back(id.to_string());
    }

    pub async fn mark_failed(&self, id: &str, error: String) {
        {
            let mut jobs = self.jobs.write().await;
            if let Some(job) = jobs.get_mut(id) {
                job.status = JobStatus::Failed(error);
                job.completed_at = Some(chrono::Utc::now().to_rfc3339());
                println!("[Queue] Job failed: {}", id);
            }
        }
        let mut fin = self.finished_order.lock().await;
        fin.push_back(id.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_queue_max_jobs_and_eviction() {
        let q = JobQueue::new(2);
        let id1 = q.submit("t1".into(), serde_json::json!({})).await.unwrap();
        let _id2 = q.submit("t2".into(), serde_json::json!({})).await.unwrap();

        // 3rd pending job must be rejected because max_jobs == 2
        assert!(q.submit("t3".into(), serde_json::json!({})).await.is_err());

        // Complete first job -> new submission evicts completed record and succeeds!
        let popped = q.dequeue().await.unwrap();
        assert_eq!(popped, id1);
        q.mark_completed(&id1, serde_json::json!({"ok": true})).await;

        let id3 = q.submit("t3".into(), serde_json::json!({})).await.unwrap();
        assert!(!id3.is_empty());
        assert!(q.get_status(&id1).await.is_none(), "Old completed job should be evicted at max_jobs");
    }
}
