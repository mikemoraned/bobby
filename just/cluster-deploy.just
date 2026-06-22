# Cluster workload operations: deploy, roll back, scale, restart, and tail logs
# for the in-cluster components. Provisioning (create/secrets/addons) is in
# cluster.just; image build/push is in container.just.

# --- Deploy (build+push then apply at the current GIT_HASH) ---

cluster-deploy-pruner:
    IMAGE_TAG={{ GIT_HASH }} envsubst < infra/k8s/pruner-deployment.yaml | kubectl apply -f -

cluster-deploy-live-refine:
    IMAGE_TAG={{ GIT_HASH }} envsubst < infra/k8s/live-refine-deployment.yaml | kubectl apply -f -

cluster-deploy-skeet-publish:
    IMAGE_TAG={{ GIT_HASH }} envsubst < infra/k8s/skeet-publish-deployment.yaml | kubectl apply -f -

cluster-deploy-optimise:
    IMAGE_TAG={{ GIT_HASH }} envsubst < infra/k8s/optimise-cronjob.yaml | kubectl apply -f -

cluster-deploy-cloudflare-operations-exporter:
    IMAGE_TAG={{ GIT_HASH }} envsubst < infra/k8s/cloudflare-operations-exporter-cronjob.yaml | kubectl apply -f -

cluster-deploy-cloudflare-storage-exporter:
    IMAGE_TAG={{ GIT_HASH }} envsubst < infra/k8s/cloudflare-storage-exporter-cronjob.yaml | kubectl apply -f -

cluster-deploy-openai-cost-exporter:
    IMAGE_TAG={{ GIT_HASH }} envsubst < infra/k8s/openai-cost-exporter-cronjob.yaml | kubectl apply -f -

cluster-deploy-all: push-pruner push-live-refine push-skeet-publish push-optimise push-cloudflare-exporter push-openai-exporter cluster-deploy-pruner cluster-deploy-live-refine cluster-deploy-skeet-publish cluster-deploy-optimise cluster-deploy-cloudflare-operations-exporter cluster-deploy-cloudflare-storage-exporter cluster-deploy-openai-cost-exporter

# --- Rollback (re-render at an explicit tag so `image:` and OTEL service.version
# stay in sync; use to roll forward or back to a specific previously-pushed image) ---

cluster-rollback-pruner image_tag:
    IMAGE_TAG={{ image_tag }} envsubst < infra/k8s/pruner-deployment.yaml | kubectl apply -f -
    kubectl rollout status deployment pruner -n {{ NAMESPACE }}

cluster-rollback-live-refine image_tag:
    IMAGE_TAG={{ image_tag }} envsubst < infra/k8s/live-refine-deployment.yaml | kubectl apply -f -
    kubectl rollout status deployment live-refine -n {{ NAMESPACE }}

cluster-rollback-skeet-publish image_tag:
    IMAGE_TAG={{ image_tag }} envsubst < infra/k8s/skeet-publish-deployment.yaml | kubectl apply -f -
    kubectl rollout status deployment skeet-publish -n {{ NAMESPACE }}

# --- Scale & restart ---

cluster-restart-all: cluster-restart-pruner cluster-restart-live-refine cluster-restart-skeet-publish

cluster-enable-all: cluster-enable-pruner cluster-enable-live-refine cluster-enable-skeet-publish

cluster-disable-all: cluster-disable-pruner cluster-disable-live-refine cluster-disable-skeet-publish

cluster-restart-pruner:
    kubectl rollout restart deployment pruner -n {{ NAMESPACE }}

cluster-restart-live-refine:
    kubectl rollout restart deployment live-refine -n {{ NAMESPACE }}

cluster-restart-skeet-publish:
    kubectl rollout restart deployment skeet-publish -n {{ NAMESPACE }}

cluster-enable-pruner:
    kubectl scale deployment pruner --replicas 1 -n {{ NAMESPACE }}

cluster-enable-live-refine:
    kubectl scale deployment live-refine --replicas 1 -n {{ NAMESPACE }}

cluster-enable-skeet-publish:
    kubectl scale deployment skeet-publish --replicas 1 -n {{ NAMESPACE }}

cluster-disable-pruner:
    kubectl scale deployment pruner --replicas 0 -n {{ NAMESPACE }}

cluster-disable-live-refine:
    kubectl scale deployment live-refine --replicas 0 -n {{ NAMESPACE }}

cluster-disable-skeet-publish:
    kubectl scale deployment skeet-publish --replicas 0 -n {{ NAMESPACE }}

# --- Logs (tailed through lnav) ---

cluster-logs-pruner:
    kubectl logs -l app=pruner --tail=100 -f -n {{ NAMESPACE }} | lnav

cluster-logs-live-refine:
    kubectl logs -l app=live-refine --tail=100 -f -n {{ NAMESPACE }} | lnav

cluster-logs-skeet-publish:
    kubectl logs -l app=skeet-publish --tail=100 -f -n {{ NAMESPACE }} | lnav

cluster-logs-optimise:
    kubectl logs -l app=optimise --tail=100 -f -n {{ NAMESPACE }} | lnav

cluster-logs-cloudflare-operations-exporter:
    kubectl logs -l app=cloudflare-operations-exporter --tail=100 -f -n {{ NAMESPACE }} | lnav

cluster-logs-cloudflare-storage-exporter:
    kubectl logs -l app=cloudflare-storage-exporter --tail=100 -f -n {{ NAMESPACE }} | lnav

cluster-logs-openai-cost-exporter:
    kubectl logs -l app=openai-cost-exporter --tail=100 -f -n {{ NAMESPACE }} | lnav

cluster-status:
    kubectl get deployments,cronjobs,jobs,pods -n {{ NAMESPACE }}
