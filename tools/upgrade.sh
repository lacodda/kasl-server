#!/bin/sh
# Upgrades an installation made from docker-compose.install.yml to another
# release: the data copied aside first, then the version pinned in .env, the
# image pulled and the server recreated, and a wait until it answers as the
# new version with its database.
#
#   sh upgrade.sh 0.32.1              # in the installation's directory
#   sh upgrade.sh 0.32.1 /srv/kasl    # or naming it
#
# The copy is the server's own backup, written to backups/ beside .env. A
# release migrates the database when it starts, and an older image does not
# start on a newer schema, so the way back from a bad release is that file
# restored into an empty database - not the old number put back in .env.
set -eu

say() { echo "upgrade: $*"; }
fail() {
    echo "upgrade: $*" >&2
    exit 1
}

[ "$#" -ge 1 ] || fail "usage: sh upgrade.sh VERSION [DIRECTORY]"
version=${1#v}
case "$version" in
    [0-9]*.[0-9]*.[0-9]*) ;;
    *) fail "'$1' is not a version such as 0.32.1" ;;
esac
cd "${2:-.}" || fail "no directory ${2:-.}"
here=$(pwd)

[ -f .env ] || fail "no .env in $here: an installation keeps its password there, so this is not one"
# An installation whose file has another name says so with COMPOSE_FILE in
# .env, which docker compose reads. A directory whose compose file has no
# server - a development database, say - is the wrong one to pull into.
docker compose config --services 2>/dev/null | grep -qx server ||
    fail "the compose file in $here has no server service; name the installation's file with COMPOSE_FILE in .env"

if command -v curl >/dev/null 2>&1; then
    fetch() { curl -fsS --max-time 3 "$1"; }
elif command -v wget >/dev/null 2>&1; then
    fetch() { wget -qO- -T 3 "$1"; }
else
    fail "neither curl nor wget is here to ask the server whether it is up"
fi

setting() { sed -n "s/^$1=//p" .env | tail -n 1; }
port=$(setting KASL_PORT)
port=${port:-8080}
before=$(setting KASL_VERSION)
before=${before:-latest}

umask 077
mkdir -p backups
backup="backups/pre-$version-$(date -u +%Y%m%dT%H%M%SZ).jsonl"
if [ -n "$(docker compose ps -q server 2>/dev/null)" ]; then
    # Written aside and renamed once whole, so a file under that name is
    # always a complete backup.
    if ! docker compose exec -T server kasl-server backup >"$backup.part"; then
        rm -f "$backup.part"
        fail "the backup failed, so nothing was changed"
    fi
    mv "$backup.part" "$backup"
    say "the data copied aside to $here/$backup"
else
    say "no server is running in $here, so there is nothing to copy aside"
fi

# .env is rewritten whole and renamed into place: a half-written file would
# lose the database password with it.
pinned=$(mktemp .env.XXXXXX)
grep -v '^KASL_VERSION=' .env >"$pinned" || true
printf 'KASL_VERSION=%s\n' "$version" >>"$pinned"
mv "$pinned" .env

docker compose pull server
docker compose up -d

answers() {
    health=$(fetch "http://127.0.0.1:$port/health" 2>/dev/null) || return 1
    case "$health" in *'"status":"ok"'*) ;; *) return 1 ;; esac
    case "$health" in *"\"version\":\"$version\""*) return 0 ;; *) return 1 ;; esac
}
tries=0
until answers; do
    tries=$((tries + 1))
    [ "$tries" -lt 90 ] ||
        fail "the server has not answered as $version in 90 seconds; \`docker compose logs server\` says why, and the data from before is $here/$backup"
    sleep 1
done
say "$before -> $version, answering on port $port"
