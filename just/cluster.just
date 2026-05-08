HETZNER_SSH_DIR := "infra/ssh"
export KUBECONFIG := "/Users/mxm/Code/mine/bobby_worktrees/bobby/infra/kubeconfig" # cluster-creation/deletion sits on main branch

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

# --- Addons ---

cluster-install-metrics-server:
    helm repo add metrics-server https://kubernetes-sigs.github.io/metrics-server/
    helm repo update metrics-server
    helm upgrade --install metrics-server metrics-server/metrics-server \
        --namespace kube-system \
        --set args={--kubelet-insecure-tls}

# --- Secrets ---

cluster-1password-secrets-install:
    helm repo add 1password https://1password.github.io/connect-helm-charts
    helm repo update
    op document get "bobby-connect-credentials" --vault Dev --out-file infra/1password-credentials.json --force
    helm upgrade --install connect 1password/connect \
        --set-file connect.credentials=infra/1password-credentials.json \
        --set operator.create=true \
        --set "operator.token.value=$(op read 'op://Dev/bobby-connect-token/credential')"
    rm infra/1password-credentials.json
    kubectl apply -f infra/k8s/onepassword-items.yaml


cluster-ghcr-secret-install:
    kubectl create secret docker-registry ghcr-pull-secret \
        --docker-server=ghcr.io \
        --docker-username=mikemoraned \
        "--docker-password=$(op read 'op://Dev/bobby-ghcr-pat-1/password')" \
        --dry-run=client -o yaml | kubectl apply -f -

cluster-secret-install: cluster-1password-secrets-install cluster-ghcr-secret-install 

cluster-secret-status:
    kubectl get onepassworditems
    kubectl get secrets


# --- Deployments ---

cluster-deploy-pruner:
    IMAGE_TAG={{ GIT_HASH }} envsubst < infra/k8s/pruner-deployment.yaml | kubectl apply -f -

cluster-deploy-live-refine:
    IMAGE_TAG={{ GIT_HASH }} envsubst < infra/k8s/live-refine-deployment.yaml | kubectl apply -f -

cluster-deploy-optimise:
    IMAGE_TAG={{ GIT_HASH }} envsubst < infra/k8s/optimise-cronjob.yaml | kubectl apply -f -

cluster-undeploy-compact:
    kubectl delete cronjob compact

cluster-deploy-cloudflare-operations-exporter:
    IMAGE_TAG={{ GIT_HASH }} envsubst < infra/k8s/cloudflare-operations-exporter-cronjob.yaml | kubectl apply -f -

cluster-deploy-cloudflare-storage-exporter:
    IMAGE_TAG={{ GIT_HASH }} envsubst < infra/k8s/cloudflare-storage-exporter-cronjob.yaml | kubectl apply -f -

# Re-render and re-apply with an explicit image tag, so both `image:` and
# `service.version` in OTEL_RESOURCE_ATTRIBUTES are kept in sync. Use this to
# roll forward or back to a specific previously-pushed image.
cluster-rollback-pruner image_tag:
    IMAGE_TAG={{ image_tag }} envsubst < infra/k8s/pruner-deployment.yaml | kubectl apply -f -
    kubectl rollout status deployment pruner

cluster-rollback-live-refine image_tag:
    IMAGE_TAG={{ image_tag }} envsubst < infra/k8s/live-refine-deployment.yaml | kubectl apply -f -
    kubectl rollout status deployment live-refine

cluster-logs-optimise:
    kubectl logs -l app=optimise --tail=100 -f | lnav

cluster-logs-cloudflare-operations-exporter:
    kubectl logs -l app=cloudflare-operations-exporter --tail=100 -f | lnav

cluster-logs-cloudflare-storage-exporter:
    kubectl logs -l app=cloudflare-storage-exporter --tail=100 -f | lnav

cluster-deploy-all: push-pruner push-live-refine push-optimise push-cloudflare-exporter cluster-deploy-pruner cluster-deploy-live-refine cluster-deploy-optimise cluster-deploy-cloudflare-operations-exporter cluster-deploy-cloudflare-storage-exporter

cluster-restart-all: cluster-restart-pruner cluster-restart-live-refine

cluster-enable-all: cluster-enable-pruner cluster-enable-live-refine

cluster-disable-all: cluster-disable-pruner cluster-disable-live-refine


cluster-logs-pruner:
    kubectl logs -l app=pruner --tail=100 -f | lnav

cluster-logs-live-refine:
    kubectl logs -l app=live-refine --tail=100 -f | lnav

cluster-restart-pruner:
    kubectl rollout restart deployment pruner

cluster-enable-pruner:
    kubectl scale deployment pruner --replicas 1

cluster-enable-live-refine:
    kubectl scale deployment live-refine --replicas 1

cluster-disable-pruner:
    kubectl scale deployment pruner --replicas 0

cluster-disable-live-refine:
    kubectl scale deployment live-refine --replicas 0

cluster-restart-live-refine:
    kubectl rollout restart deployment live-refine

cluster-status:
    kubectl get deployments,cronjobs,jobs,pods
