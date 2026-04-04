HETZNER_SSH_DIR := "infra/ssh"
export KUBECONFIG := "infra/kubeconfig"

cluster-prerequisites:
    brew install vitobotta/tap/hetzner_k3s helm kubectl

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

cluster-deploy: cluster-deploy-pruner cluster-deploy-live-refine

cluster-logs-pruner:
    kubectl logs -l app=pruner --tail=100 -f

cluster-logs-live-refine:
    kubectl logs -l app=live-refine --tail=100 -f

cluster-restart-pruner:
    kubectl rollout restart deployment pruner

cluster-restart-live-refine:
    kubectl rollout restart deployment live-refine

cluster-status:
    kubectl get pods
