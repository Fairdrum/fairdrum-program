#!/usr/bin/env bash
# Run before every publication, in this folder:   bash check-public.sh
# Fails if the folder holds anything that must never be public: key files, secret files, settings with node addresses or
# tokens, addresses of servers; or anything that is not on the list of public files; or a wrong program address.
set -u
cd "$(dirname "$0")"
bad=0; say() { echo "✘ $*"; bad=1; }
ID=5ww3w4HrQrrrPcjfxVRvjLpL5vBJqLt7M5ThsnEkkngf
# 1. only the listed files and folders
while IFS= read -r f; do
  case "$f" in
    ./README.md|./LICENSE-MIT|./Cargo.toml|./Cargo.lock|./Anchor.toml|./.gitignore|./check-public.sh|./.cargo/config.toml|./idl/fairdrum.json|./tests/local/fixtures/rich-pot-vectors.json) ;;
    ./programs/fairdrum/Cargo.toml|./programs/fairdrum/src/*.rs|./programs/fairdrum/tests/*.rs|./programs/fairdrum/tests/*/*.rs|./programs/fairdrum/tests/xver/*.bin) ;;
    ./.git/*|./target/*|./out/*|./app/test/fixtures/accounts.json) ;;   # not published: git's own folder, build output, the file a test writes
    *) say "not on the list of public files: $f" ;;
  esac
done < <(find . -type f | sort)
PUB=$(find . -type f -not -path './.git/*' -not -path './target/*' -not -path './out/*' -not -path './app/*' | sort)
# 2. file names that are never public
echo "$PUB" | grep -Ei '(^|/)(id|.*keypair.*|.*-key|root-key|keeper|fairdrum-dev|house-secret.*|.*\.config|config|state|partners)\.json$|\.enc$|\.pem$|\.key$|(^|/)\.env|\.config/solana|(^|/)(launch|deploy|admin|scripts|docs|app|e2e|tools)/' | while read -r f; do echo "✘ a file of a kind that is never published: $f"; done | grep . && bad=1
# 3. contents: a key file is a list of 64 numbers; tokens; node addresses with a key; server addresses
PRIV=${FD_PRIVATE_PATTERNS:-../fairdrum-private-patterns.txt}; [ -s "$PRIV" ] || PRIV=""
for f in $PUB; do
  [ "$f" = ./check-public.sh ] && continue
  case "$f" in *.bin) continue ;; esac
  tr -d ' \n' < "$f" | grep -Eq '^\[([0-9]{1,3},){63}[0-9]{1,3}\]$' && say "$f is a key file (64 numbers)"
  grep -EHn 'api[-_]?key=|helius|quiknode|alchemy\.com|triton|[0-9]{8,10}:[A-Za-z0-9_-]{35}|BEGIN [A-Z ]*PRIVATE KEY|ssh-(rsa|ed25519) ' "$f" | cut -c1-160 | grep . && say "$f: a token or a node address with a key (lines above)"
  # names that are private to the owner (server names, accounts) are NOT written in this published script: they are read from
  # a list kept outside the published folder (one extended regular expression per line)
  [ -n "$PRIV" ] && grep -EHn -f "$PRIV" "$f" | cut -c1-160 | grep . && say "$f: a name from the private list (lines above)"
  grep -EHno '\b([0-9]{1,3}\.){3}[0-9]{1,3}\b' "$f" | grep -Ev ':(127\.0\.0\.1|0\.0\.0\.0)$' | head -3 | grep . && say "$f: an IP address (lines above)"
done
# 4. the program address
grep -q "declare_id!(\"$ID\")" programs/fairdrum/src/lib.rs || say "programs/fairdrum/src/lib.rs does not declare the program address $ID"
grep -rIl '4CwPeg1f' . --exclude-dir=.git --exclude-dir=target --exclude=check-public.sh 2>/dev/null | grep . && say "the old placeholder address 4CwPeg1f… is still in the files above"
grep -q "\"address\": \"$ID\"" idl/fairdrum.json || say "idl/fairdrum.json does not carry the program address"
n=$(echo "$PUB" | grep -c .)
[ -n "$PRIV" ] || echo "(the owner's private list of names was not found next to this folder: that part of the check was skipped; it is run where the folder is prepared)"
[ "$bad" = 0 ] && { echo "PUBLIC-CHECK: OK — $n files, nothing forbidden, program address $ID"; exit 0; }
echo "PUBLIC-CHECK: FAILED — do NOT publish"; exit 1
