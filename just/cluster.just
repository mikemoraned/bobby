HETZNER_SSH_DIR := "infra/ssh"
export KUBECONFIG := "infra/kubeconfig"

cluster-prerequisites:
    brew install vitobotta/tap/hetzner_k3s helm kubectl lnav

_cluster-ssh-keys:
    mkdir -p {{ HETZNER_SSH_DIR }}
    op read "op://Dev/bobby-hetzner-ssh/private key?ssh-format=openssh" > {{ HETZNER_SSH_DIR }}/id_ed25519
    chmod 600 {{ HETZNER_SSH_DIR }}/id_ed25519
    op read "op://Dev/bobby-hetzner-ssh/public key" > {{ HETZNER_SSH_DIR }}/id_ed25519.pub

_cluster-ssh-keys-cleanup:
    rm -rf {{ HETZNER_SSH_DIR }}

cluster-create: _cluster-ssh-keys
    HCLOUD_TOKEN=$(op read "op://Dev/bobby-hetzner-api-token/password") hetzner-k3s create --config infra/bobby-cluster.yaml
    just _cluster-ssh-keys-cleanup

cluster-delete: _cluster-ssh-keys
    HCLOUD_TOKEN=$(op read "op://Dev/bobby-hetzner-api-token/password") hetzner-k3s delete --config infra/bobby-cluster.yaml
    just _cluster-ssh-keys-cleanup

# --- Secrets ---

cluster-secrets-install:
    helm repo add 1password https://1password.github.io/connect-helm-charts
    helm repo update
    op document get "bobby-connect-credentials" --vault Dev --out-file infra/1password-credentials.json --force
    helm install connect 1password/connect \
        --set-file connect.credentials=infra/1password-credentials.json \
        --set operator.create=true \
        --set "operator.token.value=$(op read 'op://Dev/bobby-connect-token/credential')"
    rm infra/1password-credentials.json
    kubectl apply -f infra/k8s/onepassword-items.yaml

cluster-secrets-status:
    kubectl get onepassworditems
    kubectl get secrets

# --- Deployments ---

cluster-ghcr-pull-secret:
    kubectl create secret docker-registry ghcr-pull-secret \
        --docker-server=ghcr.io \
        --docker-username=mikemoraned \
        "--docker-password=$(op read 'op://Dev/bobby-ghcr-pat-1/password')"

cluster-deploy-pruner:
    kubectl apply -f infra/k8s/pruner-deployment.yaml

cluster-deploy-live-refine:
    kubectl apply -f infra/k8s/live-refine-deployment.yaml

cluster-run-bench-firehose:
    -kubectl delete job bench-firehose
    kubectl apply -f infra/k8s/bench-firehose-job.yaml

cluster-deploy-compact:
    kubectl apply -f infra/k8s/compact-cronjob.yaml

cluster-logs-compact:
    kubectl logs -l app=compact --tail=100 -f | lnav

cluster-run-compact name="compact-manual":
    #!/usr/bin/env bash
    set -euo pipefail
    kubectl delete job {{ name }} 2>/dev/null || true
    kubectl create job {{ name }} --from=cronjob/compact
    echo "Waiting for pod to start..."
    kubectl wait --for=condition=Ready pod -l job-name={{ name }} --timeout=120s
    POD=$(kubectl get pod -l job-name={{ name }} -o jsonpath='{.items[0].metadata.name}')
    echo "Pod: $POD"
    echo "Tailing memory peak (bytes) — exits when pod dies:"
    while kubectl exec "$POD" -- cat /sys/fs/cgroup/memory.peak 2>/dev/null; do sleep 1; done

cluster-deploy-all: push-pruner push-live-refine push-compact cluster-deploy-pruner cluster-deploy-live-refine cluster-deploy-compact

cluster-restart-all: cluster-restart-pruner cluster-restart-live-refine

cluster-logs-pruner:
    kubectl logs -l app=pruner --tail=100 -f | lnav

cluster-logs-live-refine:
    kubectl logs -l app=live-refine --tail=100 -f | lnav

cluster-logs-bench-firehose:
    kubectl logs -l app=bench-firehose

cluster-restart-pruner:
    kubectl rollout restart deployment pruner

cluster-restart-live-refine:
    kubectl rollout restart deployment live-refine

cluster-status:
    kubectl get deployments,cronjobs,jobs,pods
