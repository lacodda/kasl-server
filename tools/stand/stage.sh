#!/bin/sh
# Assembles what `turnout deploy` uploads to a stand: the upgrade script, the
# version the stand is to run, and the one command that puts them to work.
# On the stand, naming the installations to move:
#
#   sh stand.sh /path/to/installation [/path/to/another]
#
# Nothing is built here and no source travels: an installation runs the
# release's image from the registry, which the release has published before
# the deploy starts. The upload lands in a directory of its own rather than
# in an installation, so a deploy rolled back restores these three files and
# never an installation's .env.
#
# Taken from HEAD, not the working tree: what reaches a stand is a committed
# state, and the version is the one the manifest names - a released one.
set -eu

out=${1:-deploy}
rm -rf "$out"
mkdir -p "$out"
out=$(cd "$out" && pwd)
# git is asked from inside the repository rather than handed its path: on
# Windows a path given to git.exe from a POSIX shell is not always translated.
cd "$(dirname "$0")/../.."

git show HEAD:tools/upgrade.sh >"$out/upgrade.sh"
version=$(git show HEAD:Cargo.toml | sed -n 's/^version = "\(.*\)"$/\1/p' | head -n 1)
case "$version" in
    [0-9]*) ;;
    *)
        echo "Cannot read the version from HEAD:Cargo.toml" >&2
        exit 1
        ;;
esac
echo "$version" >"$out/VERSION"
cat >"$out/stand.sh" <<'STAND'
#!/bin/sh
# Moves every installation named onto the version VERSION names, one after
# another, stopping at the first that fails.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
[ "$#" -gt 0 ] || { echo "usage: sh stand.sh INSTALLATION..." >&2; exit 2; }
for installation in "$@"; do
    sh "$here/upgrade.sh" "$(cat "$here/VERSION")" "$installation"
done
STAND
echo "staged kasl-server $version into $out/"
