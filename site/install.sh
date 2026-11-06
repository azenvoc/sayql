#!/bin/sh
# Install or update the SayQL for teams server with Docker.
#
#   curl -fsSL https://sayql.dev/install | sh
#
# It asks how your team reaches the server: your own domain, a free address made from this
# machine's IP (203-0-113-5.sslip.io), or only this machine to try it first. With a domain, Caddy
# gets an HTTPS certificate for it automatically (the name must point at this machine, with ports
# 80 and 443 open); the installer checks both and says what to fix. Run it again any time to update.
# Settings, without the questions: SAYQL_DOMAIN, SAYQL_DIR (default ~/sayql), SAYQL_ADMIN_EMAIL.
# Behind your own HTTPS proxy: SAYQL_BEHIND_PROXY=1 serves the server on port 8080, with no Caddy.
# The checks ask sayql.dev/api/setup-check, sending only the domain; SAYQL_NO_CHECK=1 skips them.
set -eu

VERSION=0.6.0
IMAGE="ghcr.io/azenvoc/sayql-server:$VERSION"
CADDY_IMAGE="caddy:2.10"
DIR="${SAYQL_DIR:-$HOME/sayql}"
CHECK_URL="${SAYQL_CHECK_URL:-https://sayql.dev/api/setup-check}"

say() { printf '%s\n' "$*"; }
fail() { printf '\nerror: %s\n' "$*" >&2; exit 1; }
ask() {
	# Read from the terminal even when this script arrives through a pipe.
	if [ -n "$TTY" ]; then
		printf '%s' "$1" >/dev/tty 2>/dev/null || true
		read -r REPLY </dev/tty || REPLY=""
	else
		REPLY=""
	fi
}
if (: </dev/tty) 2>/dev/null; then TTY=1; else TTY=""; fi
clean_domain() {
	printf '%s' "$1" | tr -d '[:space:]' | tr 'A-Z' 'a-z' | sed -e 's#^https\{0,1\}://##' -e 's#/.*$##'
}

# How this machine looks from the internet: CHECK holds sayql.dev's key=value answer, or nothing
# when it can't be asked. With a domain: where it points; with a second argument, whether ports 80
# and 443 answer there too.
check() {
	CHECK=""
	[ "${SAYQL_NO_CHECK:-}" = 1 ] && return 0
	command -v curl >/dev/null 2>&1 || return 0
	CHECK=$(curl -4 -fsS --max-time 20 "$CHECK_URL${1:+?domain=$1}${2:+&probe=1}" 2>/dev/null) || CHECK=""
}
field() { printf '%s\n' "$CHECK" | sed -n "s/^$1=//p" | head -n 1; }

# Before starting: wait until the domain points at this machine, saying which record to add.
wait_for_dns() {
	while :; do
		check "$DOMAIN"
		[ -n "$CHECK" ] || return 0
		IP=$(field ip)
		if [ "$(field points_here)" = yes ]; then
			say "OK: $DOMAIN points at this machine ($IP)."
			return 0
		fi
		HAVE=$(field addresses)
		say ""
		if [ -z "$HAVE" ]; then
			say "$DOMAIN doesn't point anywhere yet."
		else
			say "$DOMAIN points at $HAVE, not at this machine ($IP)."
		fi
		say "Add this record where your domain's DNS is managed (Cloudflare, GoDaddy, Route 53, ...):"
		say ""
		say "    Type: A    Name: $DOMAIN    Value: $IP"
		say ""
		say "Some providers want only the first part of the name, such as ${DOMAIN%%.*}."
		say "On Cloudflare, set it to DNS only (grey cloud). Changes usually show up within minutes."
		[ -n "$TTY" ] || return 0
		ask "Press Enter to check again, or type skip to carry on anyway: "
		[ "$REPLY" = skip ] && return 0
	done
}

# After starting: wait for HTTPS to work from the internet, then say what's wrong if it doesn't.
wait_for_https() {
	i=0
	while [ $i -lt 12 ]; do
		check "$DOMAIN" probe
		if [ -z "$CHECK" ]; then
			HTTPS_OK=unknown
			return 0
		fi
		if [ "$(field https)" = yes ]; then
			HTTPS_OK=1
			return 0
		fi
		i=$((i + 1))
		sleep 5
	done
	say ""
	if [ "$(field points_here)" != yes ]; then
		say "Not reachable yet: $DOMAIN doesn't point at this machine ($(field ip)). Add the A record"
		say "for it, then run this again."
	elif [ "$(field http)" != yes ]; then
		say "Not reachable yet: port 80 on this machine doesn't answer from the internet. Open ports"
		say "80 and 443 in its firewall and in your cloud's security group (or forward them on your"
		say "router), then run this again."
	else
		say "Not reachable yet: HTTPS on port 443 isn't working. Open port 443 like port 80, then run"
		say "this again. To see why the certificate failed: cd $DIR && docker compose logs caddy"
	fi
}

command -v docker >/dev/null 2>&1 ||
	fail "Docker isn't installed. Install it (https://docs.docker.com/get-docker/) and run this again."
docker compose version >/dev/null 2>&1 ||
	fail "Docker Compose v2 is missing. Update Docker (https://docs.docker.com/compose/install/) and run this again."
docker info >/dev/null 2>&1 ||
	fail "Can't reach Docker. Start it, or run this as a user that can use Docker (or with sudo)."

mkdir -p "$DIR"
cd "$DIR"

# Keep the answers from last time.
if [ -f .env ]; then
	# shellcheck disable=SC1091
	. ./.env
fi
DOMAIN="${SAYQL_DOMAIN:-}"
ADMIN_EMAIL="${SAYQL_ADMIN_EMAIL:-}"
PROXY="${SAYQL_BEHIND_PROXY:-}"
if [ ! -f .env ]; then
	say ""
	say "SayQL for teams: server setup in $DIR"
	if [ -z "$DOMAIN" ]; then
		check
		IP=$(field ip)
		FREE=""
		[ -n "$IP" ] && FREE="$(printf '%s' "$IP" | tr . -).sslip.io"
		say ""
		say "How will your team reach this server?"
		say "  1) Your own domain, such as sayql.yourcompany.com. Best for a company."
		[ -n "$FREE" ] && say "  2) A free address for this machine, $FREE. No DNS to set up."
		say "  3) Only this machine, to try SayQL first."
		say "With 1 or 2, open ports 80 and 443 to this machine; the HTTPS certificate is automatic."
		ask "Choose 1, 2 or 3, or type your domain [3]: "
		CHOICE=$(clean_domain "$REPLY")
		case "$CHOICE" in
		*.*) DOMAIN="$CHOICE" ;;
		1)
			ask "Your domain, such as sayql.yourcompany.com: "
			DOMAIN=$(clean_domain "$REPLY")
			;;
		2) DOMAIN="$FREE" ;;
		esac
	fi
	if [ -z "$ADMIN_EMAIL" ]; then
		ask "Your email, for the admin account (optional): "
		ADMIN_EMAIL=$(printf '%s' "$REPLY" | tr -d '[:space:]')
	fi
	umask 077
	{
		say "# SayQL server settings. Edit, then run: docker compose up -d"
		say "SAYQL_DOMAIN=$DOMAIN"
		say "SAYQL_ADMIN_EMAIL=$ADMIN_EMAIL"
		say "# Paste a licence key on the admin page, or set it here."
		say "SAYQL_LICENSE="
		if [ -n "$PROXY" ]; then say "SAYQL_BEHIND_PROXY=$PROXY"; fi
	} >.env
	umask 022
fi

if [ -n "$DOMAIN" ] && [ -z "$PROXY" ]; then
	wait_for_dns
fi

if [ -n "$DOMAIN" ] && [ -n "$PROXY" ]; then
	URL="https://$DOMAIN"
	cat >compose.yml <<EOF
# Written by https://sayql.dev/install. Settings live in .env next to this file.
# Your own HTTPS proxy sends https://\${SAYQL_DOMAIN} to port 8080 on this machine.
services:
  sayql:
    image: $IMAGE
    restart: unless-stopped
    # A database on this machine is at host.docker.internal.
    extra_hosts:
      - "host.docker.internal:host-gateway"
    ports:
      - "8080:8080"
    environment:
      SAYQL_PUBLIC_URL: https://\${SAYQL_DOMAIN}
      SAYQL_ADMIN_EMAIL: \${SAYQL_ADMIN_EMAIL:-}
      SAYQL_LICENSE: \${SAYQL_LICENSE:-}
    volumes:
      - sayql-data:/data
volumes:
  sayql-data:
EOF
elif [ -n "$DOMAIN" ]; then
	URL="https://$DOMAIN"
	cat >compose.yml <<EOF
# Written by https://sayql.dev/install. Settings live in .env next to this file.
services:
  sayql:
    image: $IMAGE
    restart: unless-stopped
    # A database on this machine is at host.docker.internal.
    extra_hosts:
      - "host.docker.internal:host-gateway"
    environment:
      SAYQL_PUBLIC_URL: https://\${SAYQL_DOMAIN}
      SAYQL_ADMIN_EMAIL: \${SAYQL_ADMIN_EMAIL:-}
      SAYQL_LICENSE: \${SAYQL_LICENSE:-}
    volumes:
      - sayql-data:/data
  # HTTPS with an automatic certificate for SAYQL_DOMAIN.
  caddy:
    image: $CADDY_IMAGE
    restart: unless-stopped
    command: caddy reverse-proxy --from \${SAYQL_DOMAIN} --to sayql:8080
    ports:
      - "80:80"
      - "443:443"
    volumes:
      - caddy-data:/data
volumes:
  sayql-data:
  caddy-data:
EOF
else
	URL="http://localhost:8080"
	cat >compose.yml <<EOF
# Written by https://sayql.dev/install. Settings live in .env next to this file.
# This runs on this machine only. For your team, run the installer again and pick an address:
#   rm .env && curl -fsSL https://sayql.dev/install | sh
# (Your data is kept: it's in the sayql-data volume.)
services:
  sayql:
    image: $IMAGE
    restart: unless-stopped
    # A database on this machine is at host.docker.internal.
    extra_hosts:
      - "host.docker.internal:host-gateway"
    ports:
      - "127.0.0.1:8080:8080"
    environment:
      SAYQL_PUBLIC_URL: http://localhost:8080
      SAYQL_ADMIN_EMAIL: \${SAYQL_ADMIN_EMAIL:-}
      SAYQL_LICENSE: \${SAYQL_LICENSE:-}
    volumes:
      - sayql-data:/data
volumes:
  sayql-data:
EOF
fi

say ""
say "Starting SayQL server ${VERSION}..."
docker compose pull --quiet --ignore-pull-failures
# Recreated so the logs below are from this start only.
docker compose up -d --remove-orphans --force-recreate

# The server prints a one-time admin page link until the first admin signs in.
LINK=""
i=0
while [ $i -lt 30 ]; do
	LINK=$(docker compose logs sayql 2>/dev/null | grep -o 'http[^ ]*/admin#code=[A-Za-z0-9-]*' | tail -n 1 || true)
	[ -n "$LINK" ] && break
	if docker compose logs sayql 2>/dev/null | grep -q "listening"; then
		break
	fi
	i=$((i + 1))
	sleep 1
done

HTTPS_OK=""
if [ -n "$DOMAIN" ] && [ -z "$PROXY" ]; then
	wait_for_https
fi

say ""
if [ -n "$LINK" ]; then
	say "Done. Open this link to set up SayQL (it works once):"
	say ""
	say "    $LINK"
	say ""
	say "On the admin page: add a database, then invite your team. Each person gets a link that"
	say "opens the SayQL app (https://sayql.dev) and signs them in."
	if [ -z "$DOMAIN" ]; then
		say "This runs on this machine only. For your team: rm $DIR/.env and run this again."
	elif [ -n "$PROXY" ]; then
		say "The link works once your proxy sends https://$DOMAIN to port 8080 here."
	elif [ "$HTTPS_OK" = unknown ]; then
		say "The HTTPS certificate can take a minute on the first start."
	elif [ -z "$HTTPS_OK" ]; then
		say "The link works once $DOMAIN is reachable (see above)."
	fi
else
	say "SayQL is running at $URL/admin"
	say "Need a new admin sign-in link? Run:"
	say "    cd $DIR && docker compose exec sayql sayql-server admin invite --email you@yourcompany.com --role admin"
fi
say ""
say "Your settings and data: $DIR (back up the sayql-data volume). Update: run this again."
if [ -n "$DOMAIN" ]; then
	say "Switch address: change SAYQL_DOMAIN in $DIR/.env and run this again."
fi
