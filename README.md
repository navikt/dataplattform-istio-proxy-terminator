# istio-proxy-terminator

A sidecar container that watches its own pod and shuts down the `istio-proxy`
sidecar once a target workload container has terminated.

## Problem

When a task/job container finishes but runs alongside an Istio sidecar, the
`istio-proxy` container keeps running indefinitely since nothing tells it to
exit. This keeps the pod `Running` forever, which confuses orchestrators
(e.g. Flyte) that expect the pod to reach a terminal phase once the task is
done.

This tool solves that by watching the pod, detecting when the target
container has terminated, and explicitly calling `istio-proxy`'s
`/quitquitquit` endpoint to shut it down so the pod can complete.

## How it works

1. Fetches its own pod (retrying on transient errors) via the Kubernetes API.
2. If `istio-proxy` isn't present in the pod spec, exits immediately (nothing
   to do).
3. Watches the pod for updates until the target container's status shows
   `terminated`.
4. Calls `istio-proxy`'s `quitquitquit` endpoint (with exponential backoff)
   to request a graceful shutdown.
5. If the process itself receives `SIGTERM`/`SIGINT` before that point (i.e.
   the pod is being torn down some other way), it exits immediately —
   `istio-proxy` will already be shutting down via its own signal handler in
   that case.

## Configuration

Configured entirely via environment variables:

| Variable | Required | Default | Description |
|---|---|---|---|
| `POD_NAME` | yes | - | Name of the pod this sidecar runs in. |
| `POD_NAMESPACE` | yes | - | Namespace of the pod. |
| `TARGET_CONTAINER_NAME` | yes | - | Container whose termination triggers shutdown. |
| `QUITQUITQUIT` | no | `http://localhost:15020/quitquitquit` | istio-proxy shutdown endpoint. |
| `RUST_LOG` | no | `info` | Log level filter (e.g. `debug`, `istio_proxy_terminator=debug`). |

`POD_NAME`/`POD_NAMESPACE` are typically injected via the Kubernetes
downward API:

```yaml
env:
  - name: POD_NAME
    valueFrom:
      fieldRef:
        fieldPath: metadata.name
  - name: POD_NAMESPACE
    valueFrom:
      fieldRef:
        fieldPath: metadata.namespace
```

Requires RBAC permission to `get`/`watch` its own pod.

## Building

```sh
cargo build --release
```

## Running tests

```sh
cargo test
```

## Docker

```sh
docker build -t istio-proxy-terminator .
```

Produces a small image based on `gcr.io/distroless/cc-debian12:nonroot`.

## Running locally

Requires a valid kubeconfig/in-cluster config and the env vars above:

```sh
POD_NAME=my-pod \
POD_NAMESPACE=default \
TARGET_CONTAINER_NAME=app \
RUST_LOG=debug \
cargo run
```
