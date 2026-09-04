use std::error::Error as _;
use std::time::Duration;

use futures::TryStreamExt;
use k8s_openapi::api::core::v1::Pod;
use kube::{
    Api,
    runtime::{WatchStreamExt, watcher},
};

use crate::config::Config;

pub async fn run(
    client: &kube::Client,
    http_client: &reqwest::Client,
    cfg: &Config,
) -> anyhow::Result<()> {
    let pod = get_pod_with_retry(&client, cfg).await?;

    if !has_container(&pod, "istio-proxy") {
        tracing::info!("istio-proxy not present, exiting");
        return Ok(());
    }

    let pods: Api<Pod> = Api::namespaced(client.clone(), &cfg.pod_namespace);
    let field_selector = format!("metadata.name={}", cfg.pod_name);

    tracing::info!(pod = %cfg.pod_name, target_container = %cfg.target_container_name, "watching for target container termination");
    tracing::info!("come with me if you want to live");

    watcher(pods, watcher::Config::default().fields(&field_selector))
        .applied_objects()
        .try_take_while(|pod| {
            let terminated = is_container_terminated(pod, &cfg.target_container_name);
            async move { Ok(!terminated) }
        })
        .try_for_each(|_| async move { Ok(()) })
        .await?;

    tracing::info!(target_container = %cfg.target_container_name, "target container has terminated, attempting to stop istio-proxy");
    tracing::info!("hasta la vista, baby");

    shutdown_istio_proxy(http_client, cfg).await?;

    Ok(())
}

async fn get_pod_with_retry(client: &kube::Client, cfg: &Config) -> anyhow::Result<Pod> {
    let pods: Api<Pod> = Api::namespaced(client.clone(), &cfg.pod_namespace);
    const DELAY: Duration = Duration::from_secs(10);

    loop {
        let pod = pods.get(&cfg.pod_name).await;
        match pod {
            Ok(pod) => return Ok(pod),
            Err(kube::Error::Api(e)) if e.code == 404 => {
                return Err(anyhow::anyhow!(e).context("get pod"));
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to get pod, retrying");
                tokio::time::sleep(DELAY).await;
            }
        }
    }
}

fn has_container(pod: &Pod, name: &str) -> bool {
    match &pod.spec {
        Some(spec) => spec.containers.iter().any(|c| c.name == name),
        None => false,
    }
}

fn is_container_terminated(pod: &Pod, container_name: &str) -> bool {
    let Some(status) = &pod.status else {
        return false;
    };
    let Some(statuses) = &status.container_statuses else {
        return false;
    };
    statuses
        .iter()
        .find(|s| s.name == container_name)
        .is_some_and(|s| s.state.as_ref().is_some_and(|st| st.terminated.is_some()))
}

async fn shutdown_istio_proxy(client: &reqwest::Client, cfg: &Config) -> anyhow::Result<()> {
    const MAX_DELAY: Duration = Duration::from_secs(30);
    let mut delay = Duration::from_secs(1);

    loop {
        match client.post(&cfg.quitquitquit).send().await {
            Ok(res) if res.status().is_success() => {
                tracing::info!("successfully shutdown istio-proxy");
                tracing::info!("terminated.");
                return Ok(());
            }
            Ok(res) => {
                tracing::error!(status = %res.status(), "failed to shut down istio-proxy");
            }
            Err(e) if is_expected_shutdown_error(&e) => {
                tracing::info!("istio-proxy connection closed during shutdown");
                tracing::info!("terminated.");
                return Ok(());
            }
            Err(e) => {
                tracing::error!(error = %e, "failed to shut down istio-proxy");
            }
        }

        tracing::info!(?delay, "retrying istio-proxy shutdown");
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(MAX_DELAY);
    }
}

fn is_expected_shutdown_error(err: &reqwest::Error) -> bool {
    let mut source = err.source();
    while let Some(e) = source {
        if let Some(io_err) = e.downcast_ref::<std::io::Error>() {
            if io_err.kind() == std::io::ErrorKind::UnexpectedEof
                || io_err.kind() == std::io::ErrorKind::ConnectionReset
            {
                return true;
            }
        }
        if let Some(hyper_err) = e.downcast_ref::<hyper::Error>() {
            if hyper_err.is_incomplete_message() || hyper_err.is_closed() {
                return true;
            }
        }
        source = e.source();
    }
    false
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use k8s_openapi::api::core::v1::{
        Container, ContainerState, ContainerStateRunning, ContainerStateTerminated,
        ContainerStatus, PodSpec, PodStatus,
    };
    use tokio::net::TcpListener;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    fn test_config(quitquitquit: String) -> Config {
        Config {
            pod_name: "my-pod".into(),
            pod_namespace: "default".into(),
            target_container_name: "app".into(),
            quitquitquit,
        }
    }

    #[test]
    fn is_container_terminated_present_and_terminated() {
        let pod = Pod {
            status: Some(PodStatus {
                container_statuses: Some(vec![
                    ContainerStatus {
                        name: "istio-proxy".into(),
                        ..Default::default()
                    },
                    ContainerStatus {
                        name: "app".into(),
                        state: Some(ContainerState {
                            terminated: Some(ContainerStateTerminated::default()),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                ]),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(is_container_terminated(&pod, "app"));
    }

    #[test]
    fn is_container_terminated_present_and_running() {
        let pod = Pod {
            status: Some(PodStatus {
                container_statuses: Some(vec![ContainerStatus {
                    name: "app".into(),
                    state: Some(ContainerState {
                        running: Some(ContainerStateRunning::default()),
                        ..Default::default()
                    }),
                    ..Default::default()
                }]),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(!is_container_terminated(&pod, "app"));
    }

    #[test]
    fn is_container_terminated_absent() {
        let pod = Pod {
            status: Some(PodStatus {
                container_statuses: Some(vec![ContainerStatus {
                    name: "istio-proxy".into(),
                    ..Default::default()
                }]),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(!is_container_terminated(&pod, "app"));
    }

    #[test]
    fn is_container_terminated_no_statuses() {
        let pod = Pod::default();
        assert!(!is_container_terminated(&pod, "app"));
    }

    #[test]
    fn has_container_present() {
        let pod = Pod {
            spec: Some(PodSpec {
                containers: vec![
                    Container {
                        name: "app".into(),
                        ..Default::default()
                    },
                    Container {
                        name: "istio-proxy".into(),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(has_container(&pod, "istio-proxy"));
    }

    #[test]
    fn has_container_absent() {
        let pod = Pod {
            spec: Some(PodSpec {
                containers: vec![Container {
                    name: "app".into(),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(!has_container(&pod, "istio-proxy"));
    }

    #[test]
    fn has_container_empty() {
        let pod = Pod::default();
        assert!(!has_container(&pod, "istio-proxy"));
    }

    #[tokio::test]
    async fn shutdown_istio_proxy_success_first_try() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let cfg = test_config(server.uri());
        let client = reqwest::Client::new();

        shutdown_istio_proxy(&client, &cfg)
            .await
            .expect("shutdown_istio_proxy should succeed");
    }

    #[tokio::test]
    async fn shutdown_istio_proxy_retries_then_succeeds() {
        let server = MockServer::start().await;
        let calls = std::sync::Arc::new(AtomicU32::new(0));
        let calls_clone = calls.clone();

        Mock::given(method("POST"))
            .respond_with(move |_req: &wiremock::Request| {
                let n = calls_clone.fetch_add(1, Ordering::SeqCst) + 1;
                if n < 2 {
                    ResponseTemplate::new(500)
                } else {
                    ResponseTemplate::new(200)
                }
            })
            .expect(2)
            .mount(&server)
            .await;

        let cfg = test_config(server.uri());
        let client = reqwest::Client::new();

        shutdown_istio_proxy(&client, &cfg)
            .await
            .expect("shutdown_istio_proxy should succeed after retrying");

        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn shutdown_istio_proxy_connection_closed_treated_as_success() {
        // wiremock can't simulate an abrupt connection close, so use a raw
        // TCP listener that accepts a connection and drops it immediately
        // without writing a response, mirroring the Go test's use of
        // http.Hijacker to close the underlying connection.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((socket, _)) => drop(socket),
                    Err(_) => break,
                }
            }
        });

        let cfg = test_config(format!("http://{addr}/quitquitquit"));
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();

        shutdown_istio_proxy(&client, &cfg)
            .await
            .expect("connection closed during shutdown should be treated as success");
    }
}
