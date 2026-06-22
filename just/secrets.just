# Generate/rotate the shared secrets in 1Password (consumed by the services via
# their `deploy/*.env` files). Idempotent: create the item, else rotate it.

generate-sse-c-key:
    op item create --vault Dev --title hom-bobby-r2-sse-c-key --category password "password=$(openssl rand -base64 32)" \
        || op item edit hom-bobby-r2-sse-c-key --vault Dev "password=$(openssl rand -base64 32)"

generate-session-secret:
    op item create --vault Dev --title bobby-session-secret --category password "password=$(openssl rand -hex 32)" \
        || op item edit bobby-session-secret --vault Dev "password=$(openssl rand -hex 32)"
