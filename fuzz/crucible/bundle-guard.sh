#!/usr/bin/env bash
# bundle-guard.sh — fail-closed checks for a crucible FuzzCorp bundle.
#
#   ./bundle-guard.sh <bundle-dir> [repo-root]      # verify (and stage missing sources)
#   CHECK_ONLY=1 ./bundle-guard.sh <bundle-dir>     # verify without staging anything
#
# Run this AFTER build-bundle.sh and BEFORE uploading. Every failure it catches is
# otherwise SILENT: the bundle validates, uploads, runs, and quietly produces
# nothing (or a fraction of what it should). None of them surface as an error, so
# the first sign is an empty dashboard days later.
#
#   GATE A  the harness actually has a fuzz test compiled in. Built without
#           `--features <feature>` the binary starts, selects no test, prints
#           "No fuzz test selected" and exits 0 -- a green campaign that fuzzed
#           nothing.
#   GATE B  every source path in the DWARF resolves to a real file in the bundle. The
#           paths are composed from the line table the way the driver composes them
#           (comp_dir + include directory + file name), then resolved
#           using the driver's OWN rule (fuzzcorp lib/coverage/lcov/lcov.go:245,
#           `Info.Replace`): a record whose key starts with sources_original_path is
#           rewritten into sources_path_in_bundle; a record that does NOT match keeps
#           its key verbatim and resolves relative to the BUNDLE ROOT. Programs
#           that span two source roots (programs/ + libraries/, program-libs/, ...)
#           lose the second one entirely unless it is staged at the bundle root.
#   GATE C  the manifest contains no absolute build-machine path. Those never match
#           on a CI-built bundle, and they leak the builder's username.
#   GATE D  the harness binary is linux/amd64. Workers are amd64; a mismatched
#           bundle validates and is then never picked up -- it fails as silence.
#           Non-static linkage is reported loudly but is not fatal, because a
#           glibc build MAY still run depending on the worker image.
#   GATE F  no COMMITTED harness source embeds a build-machine absolute path. The
#           generator emits a `SCOUT_GENERATION_PROVENANCE` constant recording the
#           absolute path of each of its inputs -- nothing reads it, and it carries
#           the operator's home directory and username into a file that ships to the
#           client. It comes back on every regeneration, so this has to be a gate
#           rather than a one-time cleanup.
#   GATE G  no COMMITTED harness source names the authoring toolchain -- its scripts,
#           its private docs, its commands. Same reason as GATE F: the generated
#           header re-appears on every regeneration, and none of those references
#           resolves to anything the client can open.
#   GATE H  the symbols file is the unstripped twin of a SHIPPED program (.text byte-identical)
#           and its DWARF line table maps to addresses inside .text. Coverage is computed
#           from the symbols and attributed to the executed program: symbols from another
#           build put every line in the wrong place, and out-of-range addresses resolve
#           0 PCs -- both silently, both reported by the worker as a path mismatch.
#   GATE I  the deploy workflow can actually upload. fuzz-upload-action takes ONLY
#           upload_type/upload_path/upload_args and reads credentials from env;
#           passing api-key:/org:/project:/bundle-dir: as inputs fails its own
#           "Validate env" step and nothing is ever uploaded, behind a green check.
#           Also requires the action pinned to a commit SHA and `timeout-minutes`.
#           Needs [repo-root]; skipped with a note when not given.
set -euo pipefail

BUNDLE="${1:?usage: bundle-guard.sh <bundle-dir> [repo-root]}"
REPO="${2:-}"
BUNDLE="$(cd "$BUNDLE" && pwd)"
[[ -n "$REPO" ]] && REPO="$(cd "$REPO" && pwd)"

HERE="$(cd "$(dirname "$0")" && pwd)"

python3 - "$BUNDLE" "$REPO" "${CHECK_ONLY:-0}" "$HERE" <<'PY'
import json, os, re, shutil, subprocess, sys

bundle, repo, check_only = sys.argv[1], sys.argv[2], sys.argv[3] == "1"
mf = os.path.join(bundle, "manifest.fc.json")
errors, warnings, notes = [], [], []

def strings(path):
    try:
        out = subprocess.run(["strings", "-a", path], capture_output=True,
                             text=True, errors="replace", timeout=1800).stdout
        if out.strip():
            return out
    except Exception:
        pass
    try:                                    # busybox / no binutils fallback
        data = open(path, "rb").read()
        return "\n".join(re.findall(rb"[\x20-\x7e]{4,}", data).__iter__().__str__()
                         for _ in [0])
    except Exception:
        return ""

if not os.path.exists(mf):
    print(f"ERROR: no manifest at {mf}", file=sys.stderr); sys.exit(1)
try:
    man = json.load(open(mf))
except json.JSONDecodeError as e:
    # A build-bundle.sh that interpolates an EMPTY shell var into its manifest
    # heredoc leaves a dangling comma or a blank key -- valid shell, invalid JSON.
    # The upload then fails with something far less obvious. Show the offending
    # region rather than a bare traceback.
    lines = open(mf, encoding="utf-8", errors="replace").read().split("\n")
    lo, hi = max(0, e.lineno - 4), min(len(lines), e.lineno + 3)
    print(f"ERROR: {mf} is not valid JSON: {e.msg} (line {e.lineno}, col {e.colno})",
          file=sys.stderr)
    print("       An empty shell variable in the manifest heredoc is the usual cause.",
          file=sys.stderr)
    for i in range(lo, hi):
        mark = ">>" if i + 1 == e.lineno else "  "
        print(f"  {mark} {i+1:4}| {lines[i]}", file=sys.stderr)
    sys.exit(1)

# ---------------------------------------------------------------- GATE C ----
raw = open(mf, encoding="utf-8").read()
tracked = subprocess.run(["git", "ls-files", "--error-unmatch", mf],
                         capture_output=True, cwd=os.path.dirname(mf) or ".").returncode == 0
for m in re.finditer(r'"([^"]*/(?:Users|home)/[^"]*)"', raw):
    lit = m.group(1)
    # An absolute sources_original_path is CORRECT when the DWARF genuinely carries the
    # build machine's path AND the manifest is regenerated next to the .so on that same
    # machine (loopscale does exactly this, deliberately). It is only a defect when the
    # manifest is a COMMITTED artifact: then it is frozen to one machine, matches nothing
    # anywhere else, and ships someone's username to the client.
    if tracked:
        errors.append(f"GATE C: COMMITTED manifest embeds an absolute build-machine path:\n"
                      f"        {lit[:110]}\n"
                      f"        Frozen to one machine: it cannot match a CI-built bundle (coverage\n"
                      f"        renders empty) and it leaks the builder's username to the client.")
    elif not os.path.exists(lit.rstrip("/")):
        errors.append(f"GATE C: manifest references an absolute path that does not exist here:\n"
                      f"        {lit[:110]}\n"
                      f"        The .so was built elsewhere, so this prefix matches nothing.")
    else:
        notes.append("GATE C: absolute sources_original_path, generated locally and not committed "
                     "(valid: it matches this machine's DWARF)")

# ---------------------------------------------------------------- GATE F ----
harness_dir = sys.argv[4] if len(sys.argv) > 4 else os.path.dirname(bundle)
tracked_src = subprocess.run(["git", "ls-files", "--", "src", "idls", "*.sh", "*.toml"],
                             capture_output=True, text=True, cwd=harness_dir)
ABS = re.compile(r'(?<![\w-])/(?:Users|home)/[A-Za-z0-9._-]+/')
for rel in tracked_src.stdout.split():
    full = os.path.join(harness_dir, rel)
    try:
        text = open(full, encoding="utf-8", errors="replace").read()
    except OSError:
        continue
    for m in ABS.finditer(text):
        line = text.count("\n", 0, m.start()) + 1
        snippet = text[m.start():m.start() + 90].split("\n")[0]
        # A path inside a comment illustrating someone ELSE's machine (a CI runner) is
        # documentation, not a leak; a real one appears in code or data.
        if "runner" in snippet:
            continue
        errors.append(f"GATE F: committed harness source embeds a build-machine path:\n"
                      f"        {rel}:{line}: {snippet}\n"
                      f"        This ships the builder's home directory and username to the\n"
                      f"        client. If it is SCOUT_GENERATION_PROVENANCE, delete the constant:\n"
                      f"        nothing reads it and regeneration re-emits it every time.")
        break

# ---------------------------------------------------------------- GATE G ----
# Same fail-closed reasoning as GATE F, for a different leak. The generator stamps a
# `GENERATED by <tool> <script>.py` header, and hand-written comments accumulate citations
# to the authoring toolchain's own private docs and commands. None of it resolves to
# anything the client can open, so it reads as a dangling reference in a file they now own.
# The header comes back on every regeneration, so it has to be a gate, not a cleanup.
TOOLING = re.compile(r'crucible-scout|references/[a-z-]+\.md|(?<![\w.])[a-z_]+\.py\b|`scout [a-z]+`')
for rel in tracked_src.stdout.split():
    # This file has to spell out what it forbids, so it cannot be subject to its own gate.
    if os.path.basename(rel) == "bundle-guard.sh":
        continue
    full = os.path.join(harness_dir, rel)
    try:
        text = open(full, encoding="utf-8", errors="replace").read()
    except OSError:
        continue
    m = TOOLING.search(text)
    if m:
        line = text.count("\n", 0, m.start()) + 1
        errors.append(f"GATE G: committed harness source names the authoring toolchain:\n"
                      f"        {rel}:{line}: {text[m.start():m.start() + 90].splitlines()[0]}\n"
                      f"        The client cannot resolve it. Describe the thing, not the tool.")

# Manifest v3 keys are snake_case. A manifest in the old PascalCase schema (or with no lineages)
# would skip every per-lineage gate below and pass vacuously, so it is a failure here.
lineages = man.get("lineages") if isinstance(man, dict) else None
if not lineages:
    stale = isinstance(man, dict) and "Lineages" in man
    errors.append("GATE A: the manifest declares no lineages -- nothing would be fuzzed or checked"
                  + (".\n        It uses the old PascalCase keys; FuzzCorp reads snake_case"
                     " (lineages, memory_kib, ...)." if stale else ""))
for lin in lineages or []:
    for conf in lin.get("confs", []):
        p = conf.get("driver", {}).get("params", {})
        name = lin.get("name", "?")
        binp  = p.get("binary_path_in_bundle")
        symp  = p.get("symbols_path_in_bundle")
        srcs  = p.get("sources_path_in_bundle")
        orig  = p.get("sources_original_path")

        # ------------------------------------------------------- GATE A/D ---
        if not binp:
            errors.append(f"GATE A [{name}]: no binary_path_in_bundle"); continue
        bpath = os.path.join(bundle, binp)
        if not os.path.exists(bpath):
            errors.append(f"GATE A [{name}]: binary missing: {binp}"); continue

        feature = os.path.basename(binp)
        if feature not in strings(bpath):
            errors.append(f"GATE A [{name}]: '{feature}' does not appear in {binp}.\n"
                          f"        The fuzz test is not registered -- the campaign would report\n"
                          f"        success while fuzzing nothing. Was --features {feature} passed?")
        else:
            notes.append(f"GATE A [{name}]: fuzz test '{feature}' registered")

        try:
            desc = subprocess.run(["file", "-b", bpath], capture_output=True,
                                  text=True, timeout=120).stdout.strip()
        except Exception:
            desc = ""
        if desc:
            if "x86-64" not in desc:
                errors.append(f"GATE D [{name}]: harness is not x86-64 -- workers are amd64 and\n"
                              f"        will never pick it up. `file` says: {desc}")
            elif not any(k in desc for k in ("statically linked", "static-pie linked")):
                warnings.append(f"GATE D [{name}]: harness is NOT statically linked. A glibc build\n"
                                f"        can die on the worker with a bare `status 1`. {desc}")
            else:
                notes.append(f"GATE D [{name}]: static linux/amd64 binary")

        # --------------------------------------------------------- GATE B ---
        if not symp:
            warnings.append(f"GATE B [{name}]: no symbols_path_in_bundle -- no source-level coverage")
            continue
        spath = os.path.join(bundle, symp)
        if not os.path.exists(spath):
            errors.append(f"GATE B [{name}]: symbols missing: {symp}"); continue
        if "/target/" in symp or symp.startswith("target/"):
            errors.append(f"GATE B [{name}]: symbols_path_in_bundle contains 'target/' ({symp}).\n"
                          f"        Crucible splits FUZZ_SYMBOLS at '/target/' to infer the source\n"
                          f"        root, so this corrupts DWARF resolution -> 0 source files.")
        if not orig:
            errors.append(f"GATE B [{name}]: no sources_original_path -> coverage renders empty"); continue

        pref = orig if orig.endswith("/") else orig + "/"

        # ------------------------------------------------------- GATE E ---
        # Reproduce the SERVER's own check before uploading. The cover task runs
        # `Info.Has(sources_original_path)` over the LCOV and fails the whole task
        # with "SourcesOriginalPath ... does not match any source file in the
        # coverage profile" (the worker's wording) when nothing matches -- which
        # surfaces as lines_found: 0 and an empty dashboard, days later.
        #
        # The LCOV keys come from the DWARF compile-unit paths, so check the
        # prefix against those directly. `strings` CANNOT do this: comp_dir is
        # stored once, apart from the relative file names, so it reconstructs
        # paths that do not exist. Use a real DWARF reader, and if none is
        # available say so rather than passing silently.
        dd = None
        for cand in ("llvm-dwarfdump", "dwarfdump"):
            if shutil.which(cand): dd = cand; break
        if dd is None:
            import glob as _g
            hits = sorted(_g.glob("/usr/bin/llvm-dwarfdump-*")
                          + _g.glob("/usr/lib/llvm-*/bin/llvm-dwarfdump"))
            dd = hits[-1] if hits else None
        if dd is None:
            warnings.append(f"GATE E [{name}]: no llvm-dwarfdump; cannot verify that\n"
                            f"        sources_original_path={orig!r} matches the coverage profile.\n"
                            f"        Install llvm (apt-get install -y llvm) to make this checkable.")
        else:
            # Read the LINE TABLE, not .debug_info. An SF: key is comp_dir (when
            # present) + include_directories[] + file_names[]. It is NOT the
            # compile unit's DW_AT_name: that carries a codegen-unit suffix
            # (.../lib.rs/@/crate.hash-cgu.00) and on real SBF artifacts differs
            # from what lands in the profile -- deriving from it produced a prefix
            # the server then rejected.
            try:
                di = subprocess.run([dd, "--debug-info", spath], capture_output=True,
                                    text=True, errors="replace", timeout=1800).stdout
                dl = subprocess.run([dd, "--debug-line", spath], capture_output=True,
                                    text=True, errors="replace", timeout=1800).stdout
            except Exception:
                di = dl = ""
            comp = {c for c in re.findall(r'DW_AT_comp_dir\s*\("([^"]*)"\)', di)
                    if not re.search(r'\.cargo|/rustc/|toolchain|bpf-tools|platform-tools', c)}
            dirs = re.findall(r'include_directories\[\s*\d+\]\s*=\s*"([^"]*)"', dl)
            keys = set()
            for d in dirs:
                if d.startswith("/"):
                    keys.add(d)
                else:
                    keys.add(d)
                    for c in comp:
                        keys.add(f"{c.rstrip('/')}/{d}")
            if not keys:
                warnings.append(f"GATE E [{name}]: no line-table directories readable from {symp};\n"
                                f"        cannot verify the coverage prefix from here.")
            elif not any(k.startswith(pref) or k.startswith(orig) for k in keys):
                sample = sorted(keys)[:4]
                errors.append(
                    f"GATE E [{name}]: sources_original_path={orig!r} matches NONE of the\n"
                    f"        {len(keys)} line-table directories in the coverage profile. The server\n"
                    f"        fail the cover task and the project renders lines_found: 0.\n"
                    f"        comp_dir={sorted(comp)[:2] or '<none>'}\n"
                    f"        actual paths e.g. {sample}")
            else:
                hit = sum(1 for k in keys if k.startswith(pref) or k.startswith(orig))
                notes.append(f"GATE E [{name}]: sources_original_path matches {hit}/{len(keys)} "
                             f"line-table directories")

        # ------------------------------------------------------- GATE H ---
        # Coverage is computed from the SYMBOLS file and attributed to the PROGRAM the
        # harness executes, so they must be the same build: an unstripped .so from an
        # earlier or differently-configured `cargo build-sbf` carries valid-looking DWARF
        # whose PCs land on the wrong lines, with no error anywhere. Stripping the deploy
        # artifact leaves .text untouched, so byte-identical .text is the proof. And the
        # line table must map to addresses INSIDE .text: an artifact can carry hundreds of
        # compile units with correct source paths and still resolve 0 PCs because every
        # address is out of range, which the worker reports as "SourcesOriginalPath ...
        # does not match" -- a path bug it is not.
        def elf_sections(path):
            import struct
            d = open(path, "rb").read()
            if d[:4] != b"\x7fELF" or d[4] != 2 or d[5] != 1:
                raise ValueError("not a little-endian ELF64 file")
            shoff = struct.unpack_from("<Q", d, 0x28)[0]
            shentsize, shnum, shstrndx = struct.unpack_from("<HHH", d, 0x3A)
            hdrs = [struct.unpack_from("<IIQQQQIIQQ", d, shoff + i * shentsize) for i in range(shnum)]
            stro = hdrs[shstrndx][4]
            out = {}
            for h in hdrs:
                nm = d[stro + h[0]:d.index(b"\0", stro + h[0])].decode(errors="replace")
                out[nm] = {"addr": h[3], "size": h[5], "data": d[h[4]:h[4] + h[5]]}
            return out
        try:
            text = elf_sections(spath).get(".text")
            if text is None:
                raise ValueError("no .text section")
        except Exception as e:
            text = None
            warnings.append(f"GATE H [{name}]: cannot read {symp} as an ELF with .text ({e}); "
                            f"symbols/program correspondence not verified")
        if text is not None:
            rundir = os.path.join(bundle, (p.get("harness_run_dir_in_bundle") or "").strip("/"))
            cands = []
            for dp, _, fns in os.walk(rundir):
                for fn in fns:
                    full = os.path.join(dp, fn)
                    if fn.endswith(".so") and os.path.abspath(full) != os.path.abspath(spath):
                        cands.append(full)
            match = None
            for c in sorted(cands):
                try:
                    ct = elf_sections(c).get(".text")
                except Exception:
                    continue
                if ct is not None and ct["data"] == text["data"]:
                    match = c
                    break
            if not cands:
                warnings.append(f"GATE H [{name}]: no program .so under "
                                f"{os.path.relpath(rundir, bundle)}/ to compare {symp} against")
            elif match is None:
                errors.append(f"GATE H [{name}]: the .text of {symp} matches NONE of the shipped programs\n"
                              f"        ({', '.join(os.path.relpath(c, bundle) for c in sorted(cands))}).\n"
                              f"        The symbols come from a different build than the program the harness\n"
                              f"        executes: PCs do not correspond and coverage lands on the wrong lines\n"
                              f"        with no error. Stage the unstripped .so from the SAME cargo build-sbf\n"
                              f"        invocation as the deploy artifact.")
            else:
                notes.append(f"GATE H [{name}]: {symp} .text is byte-identical to "
                             f"{os.path.relpath(match, bundle)} ({text['size']} bytes)")
            if dd is not None:
                lo, hi = text["addr"], text["addr"] + text["size"]
                addrs = [int(a, 16) for a in re.findall(r'^0x([0-9a-f]{16})\s+\d+\s+\d+', dl, re.M)]
                nz = [a for a in addrs if a]
                inr = [a for a in nz if lo <= a < hi]
                if not nz:
                    warnings.append(f"GATE H [{name}]: no line-table rows readable from {symp}; "
                                    f"cannot verify that DWARF addresses land in .text")
                elif not inr:
                    errors.append(f"GATE H [{name}]: 0 of {len(nz)} line-table addresses fall inside .text\n"
                                  f"        [{lo:#x}, {hi:#x}) (seen {min(nz):#x}..{max(nz):#x}). The DWARF\n"
                                  f"        resolves 0 PCs and coverage renders EMPTY; the worker reports it as\n"
                                  f"        'SourcesOriginalPath ... does not match', which it is not. Rebuild\n"
                                  f"        the symbols with a platform-tools >= v1.51 linker.")
                elif len(inr) * 2 < len(nz):
                    warnings.append(f"GATE H [{name}]: only {len(inr)}/{len(nz)} line-table addresses fall "
                                    f"inside .text [{lo:#x}, {hi:#x}); coverage may be partial")
                else:
                    notes.append(f"GATE H [{name}]: {len(inr)}/{len(nz)} line-table addresses inside .text")

        # The paths the driver keys LCOV records on are composed from the LINE TABLE: for
        # each unit, comp_dir (when present) + include_directories[dir_index] + file name.
        # That is exact for every DWARF shape -- relative keys (workspace builds, no
        # comp_dir) and absolute ones (a crate built with --disable-remap-cwd, whose file
        # names are a bare "src/<f>.rs" that a `strings` scan can never attribute to the
        # program). The `strings` scan below is only the fallback when no reader is available.
        paths = []
        if dd is not None and dl:
            comps_in_order = re.findall(r'DW_AT_comp_dir\s*\("([^"]*)"\)', di)
            unit, dirs, cur = -1, {}, None
            for ln in dl.splitlines():
                if "debug_line[" in ln:
                    unit += 1; dirs = {}; continue
                mm = re.match(r'\s*include_directories\[\s*(\d+)\]\s*=\s*"([^"]*)"', ln)
                if mm:
                    dirs[int(mm.group(1))] = mm.group(2); continue
                mm = re.match(r'\s*name:\s*"([^"]*)"', ln)
                if mm:
                    cur = mm.group(1); continue
                mm = re.match(r'\s*dir_index:\s*(\d+)', ln)
                if mm and cur is not None and cur.endswith(".rs"):
                    d = dirs.get(int(mm.group(1)), "")
                    c = comps_in_order[unit] if 0 <= unit < len(comps_in_order) else ""
                    if cur.startswith("/"):
                        full = cur
                    elif d.startswith("/"):
                        full = os.path.join(d, cur)
                    elif c:
                        full = os.path.join(c, d, cur)
                    else:
                        full = os.path.join(d, cur)
                    paths.append(os.path.normpath(full))
            paths = sorted(set(paths))
        if not paths:
            blob = strings(spath)
            paths = sorted(set(re.findall(r'[A-Za-z0-9_@./+-]*?[a-z_-]+/src/[A-Za-z0-9_/-]+\.rs', blob)))
            # keep only plausible repo-relative or orig-prefixed paths
            paths = [q for q in paths if len(q) < 300]
        if not paths:
            warnings.append(f"GATE B [{name}]: no *.rs paths recoverable from {symp} via `strings`;\n"
                            f"        cannot verify source resolution from here (not proof of a fault).")
            continue

        srcs_root = os.path.join(bundle, (srcs or "srcs").strip("/"))
        resolved = staged = unresolved = 0
        missing_examples = []
        for q in paths:
            if q.startswith(pref):
                dest = os.path.join(srcs_root, q[len(pref):])
                src_in_repo = os.path.join(repo, q[len(pref):]) if repo else None
                # absolute orig: the repo copy lives at the original location
                if os.path.isabs(orig):
                    src_in_repo = q
            else:
                # Not under the declared prefix. Only FIRST-PARTY sources matter here:
                # the DWARF is also full of Rust stdlib and crates.io paths
                # (../../platform-tools/out/rust/library/..., registry deps), which are
                # not in the repo, are never shipped, and must not be flagged.
                if q.startswith("/"):
                    # An absolute key outside the prefix is kept verbatim by the driver and
                    # looked up relative to the bundle root, where no absolute path can exist.
                    # For a dependency that is the intended drop; for a first-party file under
                    # the repo it is coverage silently lost (a second source root).
                    if repo and q.startswith(repo.rstrip("/") + "/"):
                        unresolved += 1
                        if len(missing_examples) < 5:
                            missing_examples.append(q)
                    continue
                if q.startswith("..") or not repo or not os.path.isfile(os.path.join(repo, q)):
                    continue
                dest = os.path.join(bundle, q)
                src_in_repo = os.path.join(repo, q)
            if os.path.isfile(dest):
                resolved += 1; continue
            if (not check_only) and src_in_repo and os.path.isfile(src_in_repo):
                os.makedirs(os.path.dirname(dest), exist_ok=True)
                shutil.copy2(src_in_repo, dest)
                staged += 1; resolved += 1; continue
            unresolved += 1
            if len(missing_examples) < 5:
                missing_examples.append(os.path.relpath(dest, bundle))
        line = (f"GATE B [{name}]: {resolved}/{len(paths)} DWARF source paths resolve"
                + (f" ({staged} staged now)" if staged else ""))
        if unresolved:
            errors.append(line + f"; {unresolved} UNRESOLVED -> those lines are measured and then\n"
                                 f"        silently dropped. e.g. {missing_examples}")
        else:
            notes.append(line)

# ---------------------------------------------------------------- GATE I ----
# The workflow is the one artifact whose failure is invisible in the bundle: a
# perfect bundle plus a miswired action uploads nothing, forever, with a green check.
ACTION = "fuzz-upload-action"
ALLOWED_WITH = {"upload_type", "upload_path", "upload_args"}
# These read as plausible inputs and are the documented way to fail "Validate env".
FORBIDDEN_WITH = {"api-key", "api_key", "apikey", "org", "organization", "project",
                  "bundle-dir", "bundle_dir", "bundle", "api-origin", "api_origin"}

def gate_i(repo):
    wfdir = os.path.join(repo, ".github", "workflows")
    if not os.path.isdir(wfdir):
        notes.append("GATE I: no .github/workflows in repo root; nothing to check")
        return
    files = [os.path.join(wfdir, f) for f in sorted(os.listdir(wfdir))
             if f.endswith((".yml", ".yaml"))]
    hits = 0
    for path in files:
        try:
            text = open(path, encoding="utf-8", errors="replace").read()
        except Exception:
            continue
        if ACTION not in text:
            continue
        hits += 1
        rel = os.path.relpath(path, repo)
        lines = text.split("\n")

        for i, line in enumerate(lines):
            m = re.search(r'uses:\s*\S*' + ACTION + r'@(\S+)', line)
            if not m:
                continue
            ref = m.group(1).strip().strip('"\'')

            # Pin: a floating tag silently changes the action under a green build.
            if not re.fullmatch(r'[0-9a-f]{40}', ref):
                errors.append(
                    f"GATE I [{rel}:{i+1}]: {ACTION} pinned to {ref!r}, not a 40-char commit SHA.\n"
                    f"        Pin to the release SHA (gh api repos/asymmetric-research/"
                    f"{ACTION}/releases/latest).")

            # Collect this step's body: until the next list item at <= the step's indent.
            step_indent = len(line) - len(line.lstrip())
            body = []
            for j in range(i + 1, len(lines)):
                nxt = lines[j]
                if not nxt.strip():
                    body.append(nxt); continue
                ind = len(nxt) - len(nxt.lstrip())
                if nxt.lstrip().startswith("- ") and ind <= step_indent:
                    break
                if ind <= step_indent and re.match(r'\s*\w[\w-]*:', nxt) and ind < step_indent:
                    break
                body.append(nxt)

            def block(name):
                """Keys directly under `name:` within this step."""
                out, depth = {}, None
                for k, bl in enumerate(body):
                    if re.match(r'\s*' + name + r':\s*$', bl):
                        depth = len(bl) - len(bl.lstrip())
                        for bl2 in body[k + 1:]:
                            if not bl2.strip():
                                continue
                            ind2 = len(bl2) - len(bl2.lstrip())
                            if ind2 <= depth:
                                break
                            km = re.match(r'\s*([\w.-]+)\s*:\s*(.*)$', bl2)
                            if km and ind2 == depth + 2:
                                out[km.group(1)] = km.group(2).strip()
                        break
                return out

            with_keys, env_keys = block("with"), block("env")

            bad = sorted(set(with_keys) & FORBIDDEN_WITH)
            if bad:
                errors.append(
                    f"GATE I [{rel}:{i+1}]: {ACTION} called with input(s) {bad} that the action\n"
                    f"        does not accept -- it fails its own 'Validate env' step and NOTHING\n"
                    f"        is ever uploaded, with a green check. It takes only\n"
                    f"        {sorted(ALLOWED_WITH)}; org/project/key go in `env:` as FUZZ_*.")
            unknown = sorted(set(with_keys) - ALLOWED_WITH - FORBIDDEN_WITH)
            if unknown:
                warnings.append(f"GATE I [{rel}:{i+1}]: unrecognized input(s) {unknown} on {ACTION}")
            for req in ("upload_type", "upload_path"):
                if req not in with_keys:
                    errors.append(f"GATE I [{rel}:{i+1}]: {ACTION} is missing required input `{req}`")
            if "FUZZ_API_KEY" not in env_keys:
                errors.append(
                    f"GATE I [{rel}:{i+1}]: no FUZZ_API_KEY in the step's `env:` -- the action\n"
                    f"        reads credentials from the environment, not from `with:`.")

            up = with_keys.get("upload_path", "")
            if up and repo:
                want = os.path.relpath(bundle, repo)
                if not up.startswith("$") and os.path.normpath(up) != os.path.normpath(want):
                    warnings.append(
                        f"GATE I [{rel}:{i+1}]: upload_path={up!r} but this bundle is at {want!r}.\n"
                        f"        If build-bundle.sh does not write exactly {up!r}, the upload is empty.")

        # Job/workflow-level hardening (checked once per file, per the client-PR bar).
        if not re.search(r'^\s*timeout-minutes:', text, re.M):
            errors.append(f"GATE I [{rel}]: no `timeout-minutes:` on any job. Without it a hung\n"
                          f"        run sits against the 6h default.")
        if not re.search(r'^\s*permissions:', text, re.M):
            warnings.append(f"GATE I [{rel}]: no `permissions:` block (want `contents: read`)")
        if "actions/checkout" in text and "persist-credentials: false" not in text:
            warnings.append(f"GATE I [{rel}]: checkout without `persist-credentials: false` "
                            f"(zizmor artipacked)")
    if hits:
        notes.append(f"GATE I: {hits} workflow(s) using {ACTION} checked")
    else:
        notes.append(f"GATE I: no workflow references {ACTION}; nothing to check")

if repo:
    gate_i(repo)
else:
    notes.append("GATE I: skipped -- pass [repo-root] to check the deploy workflow")

for n in notes:    print(f"  ok   {n}")
for w in warnings: print(f"  WARN {w}")
for e in errors:   print(f"  FAIL {e}", file=sys.stderr)
if errors:
    print(f"\nbundle-guard: {len(errors)} blocking problem(s)", file=sys.stderr)
    sys.exit(1)
print(f"\nbundle-guard: OK{' (' + str(len(warnings)) + ' warning(s))' if warnings else ''}")
PY