#!/usr/bin/env bash
# Build the docs for this Mac's tailnet name and serve them to the tailnet only.
# Usage: scripts/serve-tailnet.sh [start|stop|status]
set -euo pipefail

port="${RUSSET_DOCS_PORT:-4321}"
cd "$(dirname "${BASH_SOURCE[0]}")/.."

tailnet_host() {
	tailscale status --json | node -e '
		let input = "";
		process.stdin.on("data", (chunk) => (input += chunk));
		process.stdin.on("end", () => console.log(JSON.parse(input).Self.DNSName.replace(/\.$/, "")));
	'
}

case "${1:-start}" in
start)
	host="$(tailnet_host)"
	ip="$(tailscale ip -4)"
	RUSSET_DOCS_SITE="https://$host" npx astro build
	npx astro preview stop >/dev/null 2>&1 || true
	# Listen on loopback only. Tailscale Serve is the only way in from the
	# tailnet: HTTPS under the MagicDNS name, and plain HTTP by IP address.
	npx astro preview --background --host 127.0.0.1 --port "$port" --allowed-hosts "$host,$ip"
	tailscale serve --bg "http://127.0.0.1:$port"
	tailscale serve --bg --tcp 80 "tcp://127.0.0.1:$port"
	echo "Serving https://$host/ and http://$ip/ to your tailnet."
	;;
stop)
	tailscale serve --https=443 off || true
	tailscale serve --tcp=80 off || true
	npx astro preview stop || true
	;;
status)
	tailscale serve status
	npx astro preview status
	;;
*)
	echo "Usage: scripts/serve-tailnet.sh [start|stop|status]" >&2
	exit 64
	;;
esac
