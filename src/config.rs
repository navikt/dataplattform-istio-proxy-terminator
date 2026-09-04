use anyhow::Context;

pub struct Config {
    pub pod_name: String,
    pub pod_namespace: String,
    pub target_container_name: String,
    pub quitquitquit: String,
}

impl Config {
    pub fn load() -> anyhow::Result<Config> {
        Ok(Config {
            pod_name: std::env::var("POD_NAME").context("POD_NAME must be set")?,
            pod_namespace: std::env::var("POD_NAMESPACE").context("POD_NAMESPACE must be set")?,
            target_container_name: std::env::var("TARGET_CONTAINER_NAME")
                .context("TARGET_CONTAINER_NAME must be set")?,
            quitquitquit: std::env::var("QUITQUITQUIT").unwrap_or(String::from("http://localhost:15020/quitquitquit")),
        })
    }
}
