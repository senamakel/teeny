#!/usr/bin/env bash
# Put teeny.enamakel.com in front of the landing page, once.
#
#   ./deploy/web-vhost.sh          # install the vhost, issue the cert, reload
#
# Two-phase on purpose. The full vhost references a certificate that does not
# exist yet, and certbot's webroot challenge needs nginx already answering for
# the name, so the port 80 block goes in alone first, then the cert is issued,
# then the whole file replaces it. Safe to re-run: certbot keeps an existing
# cert and every other step is idempotent.
#
# Assumes DNS for the name already reaches this box (through Cloudflare or
# not). If it does not, certbot fails at the challenge and nothing is left
# half-installed: the 80-only vhost is harmless and the next run finishes.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

DEPLOY_HOST="${DEPLOY_HOST:-droid@174.138.35.76}"
DEPLOY_KEY="${DEPLOY_KEY:-$HOME/.ssh/id_rsa}"
DOMAIN="${DOMAIN:-teeny.enamakel.com}"
CONF="deploy/nginx/$DOMAIN.conf"
AVAILABLE="/etc/nginx/sites-available/$DOMAIN.conf"
ENABLED="/etc/nginx/sites-enabled/$DOMAIN.conf"

say() { printf '\n\033[1;35m==>\033[0m %s\n' "$*"; }
die() { printf '\n\033[1;31mfailed:\033[0m %s\n' "$*" >&2; exit 1; }
remote() { ssh -o BatchMode=yes -i "$DEPLOY_KEY" "$DEPLOY_HOST" "$@"; }

[ -f "$CONF" ] || die "no vhost at $CONF"

say "Copying the vhost to the box"
scp -q -i "$DEPLOY_KEY" "$CONF" "$DEPLOY_HOST:/tmp/$DOMAIN.conf"
remote "sudo -n mkdir -p /var/www/certbot"

if remote "sudo -n test -f /etc/letsencrypt/live/$DOMAIN/fullchain.pem"; then
  say "Certificate already exists; installing the full vhost"
else
  say "No certificate yet; installing the port 80 block alone"
  # Everything up to the first 443 server block.
  remote "awk '/listen 443/{exit} {print}' /tmp/$DOMAIN.conf | sed '\$d' | sudo -n tee $AVAILABLE >/dev/null"
  remote "sudo -n ln -sf $AVAILABLE $ENABLED && sudo -n nginx -t && sudo -n systemctl reload nginx" \
    || die "nginx rejected the port 80 vhost"

  say "Issuing the certificate"
  remote "sudo -n certbot certonly --webroot -w /var/www/certbot -d $DOMAIN --non-interactive --agree-tos --keep-until-expiring --register-unsafely-without-email 2>&1 | tail -5" \
    || die "certbot could not issue for $DOMAIN. Does DNS for it reach this box yet? The port 80 vhost is installed; re-run once it does."
fi

remote "sudo -n cp /tmp/$DOMAIN.conf $AVAILABLE && sudo -n ln -sf $AVAILABLE $ENABLED && sudo -n nginx -t && sudo -n systemctl reload nginx" \
  || die "nginx rejected the full vhost"

say "Checking from the box"
remote "curl -sk --max-time 5 -o /dev/null -w '%{http_code}\n' -H 'Host: $DOMAIN' https://127.0.0.1/" || true
say "Done. https://$DOMAIN"
