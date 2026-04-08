use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;

use caloron_types::agent::AgentHealth;
use caloron_types::config::CaloronConfig;
use caloron_types::dag::DagState;

/// Global daemon state shared across all async tasks.
#[derive(Clone)]
pub struct DaemonState {
    inner: Arc<RwLock<DaemonStateInner>>,
}

struct DaemonStateInner {
    pub config: CaloronConfig,
    pub dag: Option<DagState>,
    pub agent_health: HashMap<String, AgentHealth>,
}

impl DaemonState {
    pub fn new(config: CaloronConfig) -> Self {
        Self {
            inner: Arc::new(RwLock::new(DaemonStateInner {
                config,
                dag: None,
                agent_health: HashMap::new(),
            })),
        }
    }

    pub async fn config(&self) -> CaloronConfig {
        self.inner.read().await.config.clone()
    }

    pub async fn set_dag(&self, dag: DagState) {
        self.inner.write().await.dag = Some(dag);
    }

    pub async fn dag(&self) -> Option<DagState> {
        self.inner.read().await.dag.clone()
    }

    pub async fn register_agent(&self, health: AgentHealth) {
        let id = health.agent_id.clone();
        self.inner.write().await.agent_health.insert(id, health);
    }

    pub async fn unregister_agent(&self, agent_id: &str) {
        self.inner.write().await.agent_health.remove(agent_id);
    }

    pub async fn update_agent<F>(&self, agent_id: &str, f: F)
    where
        F: FnOnce(&mut AgentHealth),
    {
        let mut state = self.inner.write().await;
        if let Some(health) = state.agent_health.get_mut(agent_id) {
            f(health);
        }
    }

    pub async fn get_agent_health(&self, agent_id: &str) -> Option<AgentHealth> {
        self.inner.read().await.agent_health.get(agent_id).cloned()
    }

    pub async fn all_agent_health(&self) -> HashMap<String, AgentHealth> {
        self.inner.read().await.agent_health.clone()
    }
}
