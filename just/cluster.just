HETZNER_SSH_DIR := "infra/ssh"
export KUBECONFIG := "/Users/mxm/Code/mine/bobby_worktrees/bobby/infra/kubeconfig" # cluster-creation/deletion sits on main branch

# Namespace holding the stable production components. Per-worktree staging runs
# its *changed* components in its own namespace, sharing the backend stores.
NAMESPACE := "production"

# Cluster provisioning: lifecycle, addons, namespace, and secrets.
# Day-to-day workload operations (deploy/rollback/scale/logs) live in cluster-deploy.just.

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

# --- Namespace ---

cluster-create-namespace:
    kubectl apply -f infra/k8s/namespace.yaml

# --- Secrets ---

cluster-1password-secrets-install: cluster-create-namespace
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
        --namespace {{ NAMESPACE }} \
        --docker-server=ghcr.io \
        --docker-username=mikemoraned \
        "--docker-password=$(op read 'op://Dev/bobby-ghcr-pat-1/password')" \
        --dry-run=client -o yaml | kubectl apply -f -

cluster-secret-install: cluster-1password-secrets-install cluster-ghcr-secret-install

cluster-secret-status:
    kubectl get onepassworditems -n {{ NAMESPACE }}
    kubectl get secrets -n {{ NAMESPACE }}
