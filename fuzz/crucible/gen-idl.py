#!/usr/bin/env python3
"""Derive idls/manifest.json from the manifest program's own source.

The harness compiles its IDL in (declare_fuzz_program! reads it at build time), so a committed
IDL is a claim about the program rather than a fact about it. This script removes the claim: it
reads the roster, the account lists and the instruction-argument layouts out of the Rust source
and writes the IDL the harness builds against, so CI can regenerate it from the tree under test
and fail when the two disagree (--check).

Why not shank. The repository generates client/idl/manifest.json with shank via
client/idl/generateIdl.js, but `shank idl` cannot parse the current source at all:
state/resting_order.rs declares `pub struct OrderType(u8)` deriving ShankType, and shank 0.4.2
and 0.4.3 both refuse a tuple struct ("failed to parse fields make sure they are all named").
generateIdl.js calls shank through spawnSync without checking its status, so the failure is
silent and the committed file is whatever a much older run produced. It is demonstrably stale: it
lists 15 instructions for a 14-variant enum, including a phantom second SwapV2 at discriminant 4,
and it hoists Deposit's trader_index_hint out of DepositParams into a second top-level argument
even though the only decode site reads it as a struct field (processor/deposit.rs:25-29,45). Both
happen to put the same bytes on the wire, but they are not the same IDL. Everything else in it --
optional accounts included, and its GlobalEvict account list -- agrees with this derivation.
Nothing here writes to client/idl/ -- that file belongs to the client and is left alone.

Sources of truth, all parsed with tree-sitter (no regex over structure):
  * program/instruction.rs  -- the #[repr(u8)] ManifestInstruction enum: variant names, explicit
                               discriminants, and the #[account(..)] attribute list that gives
                               each instruction's ordered accounts and their privileges.
  * lib.rs                  -- the dispatch match, which maps each variant to its processor fn.
  * program/processor/*.rs  -- the processor fn, the `XParams::try_from_slice(data)` it decodes,
                               and the borsh layout of that struct (transitively).

Two deliberate differences from a mechanical transcription, both widening what the fuzzer can
reach rather than narrowing it:
  * OrderType is emitted as a plain u8, not as a 6-variant enum. In the program it is
    `#[repr(transparent)] struct OrderType(u8)` (state/resting_order.rs) whose borsh form is one
    byte, and values above the last named constant are rejected at runtime by
    OrderType::is_valid(). Typing it u8 lets the fuzzer produce those bytes and exercise that
    branch; an enum would make the invalid case unreachable by construction.
  * Account privileges are widened where the runtime needs more than the attribute declares
    (see WIDENED below). Only ever widened: narrowing a privilege the program requires turns a
    real missing-check bug into an apparent constraint.

Usage:
  ./gen-idl.py                 write idls/manifest.json
  ./gen-idl.py --check         exit 1 if the committed IDL differs from a fresh derivation
  ./gen-idl.py --print         write nothing, dump the derivation to stdout
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

try:
    import tree_sitter
    import tree_sitter_rust
except ImportError:  # pragma: no cover
    sys.exit("need tree_sitter and tree_sitter_rust: python3 -m pip install tree-sitter tree-sitter-rust")

HARNESS_DIR = Path(__file__).resolve().parent
PROGRAM_CRATE = HARNESS_DIR / ".." / ".." / "programs" / "manifest"
PROGRAM_ID = "MNFSTqtC93rEfYHB6hF82sKdZpUDFWkViLByLd1k1Ms"
IDL_OUT = HARNESS_DIR / "idls" / "manifest.json"

# Accounts the runtime requires more privilege on than instruction.rs declares. Keyed by
# (instruction, account) -> set of privileges to add. Each entry cites the code that needs it.
# Only additive; see the module docstring.
WIDENED: dict[tuple[str, str], set[str]] = {}

# Accounts the loader reads that the #[account(..)] attributes omit entirely. The attributes are
# documentation nothing compiles against; next_account_info is the wire order. An instruction
# whose IDL is short by one account sends a short account list and dies in its loader on every
# single call, which reads as a broken harness rather than as an IDL defect -- so the arity
# reconciliation below makes any such gap a hard error, and this table is the only way to settle
# one. Each entry cites the next_account_info call that needs it.
EXTRA_ACCOUNTS: dict[str, list[dict]] = {
    # GlobalEvictContext::load reads an 8th account after token_program:
    #   let _system_program: Program = Program::new(next_account_info(account_iter)?, ..)
    # validation/loaders.rs:1019-1021. The ManifestInstruction::GlobalEvict attributes declare
    # only 7 (program/instruction.rs:122-128), so a derivation from the attributes alone -- this
    # one -- would be short by one. The committed client IDL happens to list it, and the program's
    # own builder passes it (program/instruction_builders/global_evict_instruction.rs); it is the
    # shank attributes that are wrong.
    "GlobalEvict": [
        {
            "name": "system_program",
            "flags": set(),
            "desc": "System program",
            "because": "validation/loaders.rs:1019 reads an 8th account; instruction.rs declares 7",
        }
    ],
}

# Instructions whose declared account count legitimately differs from the number of
# next_account_info call sites in their loader. Anything NOT listed here must match exactly.
ARITY_NOTES: dict[str, str] = {
    # SwapContext::load serves both Swap and SwapV2 and chooses between them by sniffing the
    # OWNER of account 1: manifest-owned means account 1 is the market (Swap, 13 accounts),
    # anything else means account 1 is a separate `owner` signer and account 2 is the market
    # (SwapV2, 14 accounts). validation/loaders.rs:~700. The trailing base_mint /
    # token_program_quote / quote_mint / global / global_vault are then detected by content
    # (owner == spl_token/spl_token_2022, or pubkey == a token program id), not by length, so a
    # shorter account list is valid and the static call-site count covers both shapes at once.
    "Swap": "shares SwapContext with SwapV2; shape and trailing optionals are content-sniffed",
    "SwapV2": "shares SwapContext with Swap; shape and trailing optionals are content-sniffed",
    # BatchUpdateContext::load reads the 10 optional global accounts conditionally, so the static
    # call-site count is lower than the declared maximum account list.
    "BatchUpdate": "10 optional global accounts are read conditionally",
}

# Rust type -> IDL type. Anything not here and not a known struct is a hard error, so a new
# param field cannot be silently mistyped.
SCALARS = {
    "u8": "u8",
    "u16": "u16",
    "u32": "u32",
    "u64": "u64",
    "u128": "u128",
    "i8": "i8",
    "i16": "i16",
    "i32": "i32",
    "i64": "i64",
    "bool": "bool",
    "DataIndex": "u32",  # lib/src/lib.rs: pub type DataIndex = u32
    "OrderType": "u8",  # see the module docstring
    "Pubkey": "publicKey",
    "Address": "publicKey",
}

ARITY_REPORT: list[str] = []

RUST = tree_sitter.Language(tree_sitter_rust.language())
PARSER = tree_sitter.Parser(RUST)


def parse(path: Path):
    src = path.read_bytes()
    return src, PARSER.parse(src).root_node


def text(src: bytes, node) -> str:
    return src[node.start_byte : node.end_byte].decode("utf-8")


def query(node, kinds: set[str]):
    """Depth-first walk yielding nodes whose type is in kinds."""
    stack = [node]
    while stack:
        n = stack.pop()
        if n.type in kinds:
            yield n
        stack.extend(reversed(n.children))


def camel(snake: str) -> str:
    head, *rest = snake.split("_")
    return head + "".join(w[:1].upper() + w[1:] for w in rest)


# --------------------------------------------------------------------------------------------
# instruction.rs: the enum roster and each variant's ordered #[account(..)] list
# --------------------------------------------------------------------------------------------


def parse_account_attr(body: str) -> dict:
    """Parse the inside of #[account(0, writable, signer, name = "payer", desc = "Payer")].

    Tokens are order-independent apart from the leading index, which is why this reads them as a
    set of flags plus key = "value" pairs rather than positionally.
    """
    out: dict = {"index": None, "flags": set(), "name": None, "desc": None}
    depth = 0
    buf = ""
    parts = []
    in_str = False
    for ch in body:
        if ch == '"':
            in_str = not in_str
        if not in_str:
            if ch in "([":
                depth += 1
            elif ch in ")]":
                depth -= 1
            elif ch == "," and depth == 0:
                parts.append(buf)
                buf = ""
                continue
        buf += ch
    parts.append(buf)

    for raw in parts:
        tok = raw.strip()
        if not tok:
            continue
        if "=" in tok:
            key, _, val = tok.partition("=")
            key, val = key.strip(), val.strip().strip('"')
            if key in ("name", "desc"):
                out[key] = val
            continue
        if tok.isdigit():
            if out["index"] is None:
                out["index"] = int(tok)
            continue
        out["flags"].add(tok)
    return out


def read_instruction_enum(crate: Path) -> list[dict]:
    path = crate / "src" / "program" / "instruction.rs"
    src, root = parse(path)

    enum_node = None
    for item in query(root, {"enum_item"}):
        name = item.child_by_field_name("name")
        if name is not None and text(src, name) == "ManifestInstruction":
            enum_node = item
            break
    if enum_node is None:
        sys.exit(f"{path}: no `enum ManifestInstruction`")

    body = enum_node.child_by_field_name("body")
    variants = []
    pending: list[dict] = []
    docs: list[str] = []

    for child in body.children:
        if child.type == "attribute_item":
            raw = text(src, child)
            inner = raw[raw.index("(") + 1 : raw.rindex(")")] if "account(" in raw else None
            if inner is not None:
                pending.append(parse_account_attr(inner))
            continue
        if child.type == "line_comment":
            line = text(src, child)
            if line.startswith("///"):
                docs.append(line[3:].strip())
            continue
        if child.type != "enum_variant":
            continue

        name = text(src, child.child_by_field_name("name"))
        value_node = child.child_by_field_name("value")
        if value_node is None:
            sys.exit(f"{path}: variant {name} has no explicit discriminant; refusing to guess")
        disc = int(text(src, value_node))

        accounts = sorted(pending, key=lambda a: (a["index"] is None, a["index"]))
        for expected, acc in enumerate(accounts):
            if acc["index"] != expected:
                sys.exit(f"{path}: {name} account indices are not 0..n ({[a['index'] for a in accounts]})")
            if not acc["name"]:
                sys.exit(f"{path}: {name} account {expected} has no name =")

        variants.append({"name": name, "discriminant": disc, "accounts": accounts, "docs": docs})
        pending = []
        docs = []

    discs = [v["discriminant"] for v in variants]
    if sorted(discs) != list(range(len(discs))):
        sys.exit(f"{path}: discriminants are not contiguous from 0: {discs}")
    if len(set(v["name"] for v in variants)) != len(variants):
        sys.exit(f"{path}: duplicate variant names: {discs}")
    return variants


# --------------------------------------------------------------------------------------------
# lib.rs: variant -> processor fn, from the dispatch match
# --------------------------------------------------------------------------------------------


def read_dispatch(crate: Path) -> dict[str, str]:
    path = crate / "src" / "lib.rs"
    src, root = parse(path)

    out: dict[str, str] = {}
    for arm in query(root, {"match_arm"}):
        pattern = arm.child_by_field_name("pattern")
        value = arm.child_by_field_name("value")
        if pattern is None or value is None:
            continue
        pat = text(src, pattern)
        if "ManifestInstruction::" not in pat:
            continue
        variant = pat.split("ManifestInstruction::")[1].strip().strip("{}() \n")
        calls = [text(src, c.child_by_field_name("function")) for c in query(value, {"call_expression"})]
        processors = [c for c in calls if c.startswith("process_")]
        if not processors:
            continue
        # The arm body calls exactly one process_* fn; the innermost-listed one is that call.
        out[variant] = processors[-1]
    if not out:
        sys.exit(f"{path}: could not read the ManifestInstruction dispatch match")
    return out


# --------------------------------------------------------------------------------------------
# processor/*.rs: the params struct each processor decodes, and its borsh layout
# --------------------------------------------------------------------------------------------


def field_is_certora_only(src: bytes, field) -> bool:
    """True for a field gated on the certora feature (the verification-only duplicate).

    BatchUpdateParams declares `cancels`/`orders` twice -- Vec<..> under
    #[cfg(not(feature = "certora"))] and NoResizableVec<..> under #[cfg(feature = "certora")].
    The fuzzed binary is built without that feature, so the Vec arm is the real wire layout.
    """
    node = field.prev_sibling
    while node is not None and node.type in ("attribute_item", "line_comment", "block_comment"):
        if node.type == "attribute_item":
            attr = text(src, node).replace(" ", "")
            if 'feature="certora"' in attr and "not(feature=" not in attr:
                return True
        node = node.prev_sibling
    return False


def collect_structs(crate: Path) -> dict[str, dict]:
    """Every borsh struct in the program's processor + state tree, by name."""
    structs: dict[str, dict] = {}
    roots = [crate / "src" / "program" / "processor", crate / "src" / "state"]
    for root_dir in roots:
        for path in sorted(root_dir.rglob("*.rs")):
            src, root = parse(path)
            for item in query(root, {"struct_item"}):
                name_node = item.child_by_field_name("name")
                body = item.child_by_field_name("body")
                if name_node is None or body is None or body.type != "field_declaration_list":
                    continue
                name = text(src, name_node)
                fields = []
                for field in body.children:
                    if field.type != "field_declaration":
                        continue
                    if field_is_certora_only(src, field):
                        continue
                    fname = field.child_by_field_name("name")
                    ftype = field.child_by_field_name("type")
                    if fname is None or ftype is None:
                        continue
                    fields.append((text(src, fname), text(src, ftype).replace(" ", "")))
                if name not in structs:
                    structs[name] = {"fields": fields, "path": path}
    return structs


def read_params_for_processor(crate: Path, processor: str) -> str | None:
    """The `XParams` a processor decodes, from its own `XParams::try_from_slice(data)` call."""
    for path in sorted((crate / "src" / "program" / "processor").rglob("*.rs")):
        src, root = parse(path)
        for fn in query(root, {"function_item"}):
            name = fn.child_by_field_name("name")
            if name is None or text(src, name) != processor:
                continue
            body = fn.child_by_field_name("body")
            if body is None:
                continue
            for call in query(body, {"call_expression", "scoped_identifier"}):
                frag = text(src, call).replace(" ", "")
                if "::try_from_slice" in frag:
                    return frag.split("::try_from_slice")[0].split("(")[-1].strip()
            return None
    return None


def read_loader_arity(crate: Path) -> dict[str, int]:
    """next_account_info call sites per `<X>Context::load` -- the real wire arity."""
    path = crate / "src" / "validation" / "loaders.rs"
    src, root = parse(path)
    out: dict[str, int] = {}
    for imp in query(root, {"impl_item"}):
        ty = imp.child_by_field_name("type")
        body = imp.child_by_field_name("body")
        if ty is None or body is None:
            continue
        name = text(src, ty).split("<")[0].strip()
        if not name.endswith("Context"):
            continue
        for fn in query(body, {"function_item"}):
            fname = fn.child_by_field_name("name")
            if fname is None or text(src, fname) != "load":
                continue
            fbody = fn.child_by_field_name("body")
            if fbody is None:
                continue
            calls = [
                text(src, c.child_by_field_name("function"))
                for c in query(fbody, {"call_expression"})
                if c.child_by_field_name("function") is not None
            ]
            out[name] = sum(1 for c in calls if c.split("::")[-1] == "next_account_info")
    if not out:
        sys.exit(f"{path}: found no `<X>Context::load`; the arity reconciliation cannot run")
    return out


def read_context_for_processor(crate: Path, processor: str) -> str | None:
    """The `<X>Context` a processor loads, from the `<X>Context::load(..)` call in its file.

    Scoped to the whole file rather than the fn body because most processors split into a thin
    `process_x` that decodes params and a `process_x_core` that loads the context (and the
    certora build adds further wrappers). One processor file serves one instruction -- swap.rs
    serves both Swap and SwapV2, which is exactly the shared SwapContext noted in ARITY_NOTES.
    """
    for path in sorted((crate / "src" / "program" / "processor").rglob("*.rs")):
        src, root = parse(path)
        if not any(
            fn.child_by_field_name("name") is not None
            and text(src, fn.child_by_field_name("name")) == processor
            for fn in query(root, {"function_item"})
        ):
            continue
        found: set[str] = set()
        for node in query(root, {"scoped_identifier"}):
            frag = text(src, node).replace(" ", "")
            if frag.endswith("::load") and "Context" in frag:
                found.add(frag.split("::load")[0].split("::")[-1])
        if len(found) > 1:
            sys.exit(f"{path}: {processor} is in a file loading several contexts {sorted(found)}; cannot attribute arity")
        return next(iter(found)) if found else None
    return None


def reconcile_arity(crate: Path, instructions: list[dict], dispatch: dict[str, str]) -> list[str]:
    """Fail closed when the IDL's account count disagrees with what the loader actually reads.

    The shank attributes are documentation; next_account_info is the wire order. A one-account
    gap makes an instruction die in its loader on every call forever, which is indistinguishable
    from a broken harness -- so it is a build error here, settled only by EXTRA_ACCOUNTS (with a
    citation) or ARITY_NOTES (with a reason).
    """
    arity = read_loader_arity(crate)
    notes: list[str] = []
    problems: list[str] = []
    for ix in instructions:
        processor = dispatch[ix["name"]]
        ctx = read_context_for_processor(crate, processor)
        if ctx is None:
            notes.append(f"{ix['name']}: {processor} loads no *Context (no account arity to check)")
            continue
        if ctx not in arity:
            problems.append(f"{ix['name']}: {processor} loads {ctx}, which has no ::load in loaders.rs")
            continue
        declared, required = len(ix["accounts"]), arity[ctx]
        if ix["name"] in ARITY_NOTES:
            notes.append(f"{ix['name']}: {declared} declared vs {required} read -- {ARITY_NOTES[ix['name']]}")
            continue
        if declared != required:
            problems.append(
                f"{ix['name']}: declares {declared} accounts but {ctx}::load reads {required} "
                f"via next_account_info. Add the missing account(s) to EXTRA_ACCOUNTS with a "
                f"citation, or explain the difference in ARITY_NOTES."
            )
        else:
            notes.append(f"{ix['name']}: {declared} accounts, matches {ctx}::load")
    if problems:
        sys.exit("account-arity reconciliation failed:\n  " + "\n  ".join(problems))
    return notes


def idl_type(rust: str, structs: dict[str, dict], needed: set[str]) -> object:
    rust = rust.strip()
    if rust in SCALARS:
        return SCALARS[rust]
    if rust.startswith("Option<") and rust.endswith(">"):
        return {"option": idl_type(rust[len("Option<") : -1], structs, needed)}
    if rust.startswith("Vec<") and rust.endswith(">"):
        return {"vec": idl_type(rust[len("Vec<") : -1], structs, needed)}
    if rust.startswith("[") and rust.endswith("]") and ";" in rust:
        inner, _, count = rust[1:-1].rpartition(";")
        return {"array": [idl_type(inner, structs, needed), int(count)]}
    if rust in structs:
        needed.add(rust)
        return {"defined": rust}
    sys.exit(f"unmapped Rust type in an instruction argument: {rust!r} -- add it to SCALARS or fix the parse")


def build_type_defs(names: set[str], structs: dict[str, dict]) -> list[dict]:
    """Emit the transitive closure of defined types reachable from the instruction args."""
    out: list[dict] = []
    seen: set[str] = set()
    queue = sorted(names)
    while queue:
        name = queue.pop(0)
        if name in seen:
            continue
        seen.add(name)
        extra: set[str] = set()
        fields = [
            {"name": camel(fname), "type": idl_type(ftype, structs, extra)}
            for fname, ftype in structs[name]["fields"]
        ]
        out.append({"name": name, "type": {"kind": "struct", "fields": fields}})
        queue.extend(sorted(n for n in extra if n not in seen))
    return sorted(out, key=lambda t: t["name"])


# --------------------------------------------------------------------------------------------


def derive(crate: Path) -> dict:
    crate = crate.resolve()
    variants = read_instruction_enum(crate)
    dispatch = read_dispatch(crate)
    structs = collect_structs(crate)

    missing = [v["name"] for v in variants if v["name"] not in dispatch]
    if missing:
        sys.exit(f"variants with no dispatch arm in lib.rs: {missing}")

    needed: set[str] = set()
    instructions = []
    for v in variants:
        processor = dispatch[v["name"]]
        params = read_params_for_processor(crate, processor)
        args = []
        if params is not None:
            if params not in structs:
                sys.exit(f"{v['name']}: {processor} decodes {params}, which was not found in the source tree")
            needed.add(params)
            args = [{"name": "params", "type": {"defined": params}}]

        accounts = []
        for acc in list(v["accounts"]) + EXTRA_ACCOUNTS.get(v["name"], []):
            flags = set(acc["flags"]) | WIDENED.get((v["name"], acc["name"]), set())
            entry = {
                "name": camel(acc["name"]),
                "isMut": "writable" in flags,
                "isSigner": "signer" in flags,
            }
            if "optional" in flags:
                # anchor-lang-idl's legacy converter reads isOptional (camelCase, from
                # #[serde(rename_all)] on its IdlAccount); shank's own "optional" key would be
                # silently ignored, leaving every optional account required and every short
                # account list rejected.
                entry["isOptional"] = True
            if acc["desc"]:
                entry["docs"] = [acc["desc"]]
            accounts.append(entry)

        instructions.append(
            {
                "name": v["name"],
                "accounts": accounts,
                "args": args,
                # Keep the shank enum-tag form. crucible-idl-gen reads it
                # (explicit_discriminator_bytes) and a 1-byte discriminator keeps codegen on the
                # borsh/AnchorSerialize path, which is what the program's try_from_slice expects;
                # a 4-byte one would switch it to bincode, whose Vec length prefix is a u64.
                "discriminant": {"type": "u8", "value": v["discriminant"]},
                "docs": v["docs"] or None,
            }
        )
        if instructions[-1]["docs"] is None:
            del instructions[-1]["docs"]

    global ARITY_REPORT
    ARITY_REPORT = reconcile_arity(crate, instructions, dispatch)

    return {
        "version": "3.1.0",
        "name": "manifest",
        "instructions": sorted(instructions, key=lambda i: i["discriminant"]["value"]),
        "accounts": [],
        "types": build_type_defs(needed, structs),
        "errors": read_errors(crate),
        "events": [],
        "metadata": {"origin": "manifest-fuzz-gen-idl", "address": PROGRAM_ID},
    }


def read_errors(crate: Path) -> list[dict]:
    """The ManifestError enum, so a failing action can name its error rather than a bare code."""
    path = crate / "src" / "program" / "error.rs"
    if not path.exists():
        return []
    src, root = parse(path)
    out = []
    for item in query(root, {"enum_item"}):
        name_node = item.child_by_field_name("name")
        if name_node is None or "Error" not in text(src, name_node):
            continue
        body = item.child_by_field_name("body")
        if body is None:
            continue
        code = 0
        msg = None
        for child in body.children:
            if child.type == "attribute_item":
                attr = text(src, child)
                if "error(" in attr:
                    inner = attr[attr.index("error(") + 6 : attr.rindex(")")]
                    msg = inner.strip().strip('"')
                continue
            if child.type != "enum_variant":
                continue
            value_node = child.child_by_field_name("value")
            if value_node is not None:
                raw = text(src, value_node).strip()
                code = int(raw, 0) if raw.lower().startswith("0x") else int(raw)
            out.append({"code": code, "name": text(src, child.child_by_field_name("name")), "msg": msg})
            code += 1
            msg = None
        break
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--check", action="store_true", help="exit 1 if the committed IDL is not what the source derives")
    ap.add_argument("--print", dest="dump", action="store_true", help="dump to stdout, write nothing")
    ap.add_argument("--crate", default=str(PROGRAM_CRATE), help="path to the program crate")
    args = ap.parse_args()

    idl = derive(Path(args.crate))
    rendered = json.dumps(idl, indent=2) + "\n"

    if args.dump:
        sys.stdout.write(rendered)
        return 0

    if args.check:
        if not IDL_OUT.exists():
            print(f"::error::{IDL_OUT.name} is missing -- run ./gen-idl.py", file=sys.stderr)
            return 1
        if IDL_OUT.read_text() != rendered:
            print(
                f"::error::{IDL_OUT.name} does not match the program source. The harness would be "
                "built against an IDL the program no longer has. Run ./gen-idl.py and commit it.",
                file=sys.stderr,
            )
            return 1
        print(f"{IDL_OUT.name} matches the program source "
              f"({len(idl['instructions'])} instructions, {len(idl['types'])} types)")
        return 0

    IDL_OUT.parent.mkdir(parents=True, exist_ok=True)
    IDL_OUT.write_text(rendered)
    print(f"wrote {IDL_OUT} -- {len(idl['instructions'])} instructions, {len(idl['types'])} types, "
          f"{len(idl['errors'])} errors")
    for ix in idl["instructions"]:
        opt = sum(1 for a in ix["accounts"] if a.get("isOptional"))
        print(f"  {ix['discriminant']['value']:>2}  {ix['name']:<16} {len(ix['accounts'])} accounts"
              f"{f' ({opt} optional)' if opt else ''}"
              f"  args={[a['type'].get('defined', a['type']) for a in ix['args']] or '-'}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
