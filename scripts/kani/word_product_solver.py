#!/usr/bin/env python3
"""Fail-closed SMT adapter for Kani's full-width multiplication obligations.

The input is CBMC's formula from the actual Rust MIR, not a price model. Expand
bit-vector multiplication into base-2^32 products, then overapproximate each
32-by-32 product with a shared arbitrary u64, its proven upper bound, and its
zero-input implication. This forgets correlations (adds possible executions);
only UNSAT of the larger execution set proves the original formula UNSAT.

Before solving, independently discharge bit-vector certificates for every used
width: limb reconstruction, ring distribution, and narrowing partial products.
Also prove the abstraction's bounds/zero implication using stock cvc5. No
assumptions about prices, amounts, carries, or the program's answer are added.
Unknown, unsupported input, an abstract counterexample, or a timeout fails CI.
"""

import functools
import os
import subprocess
import sys

import z3

LIMB_BITS = 32
PRODUCT_BITS = 64
PRODUCT_LIMIT = ((1 << LIMB_BITS) - 1) ** 2


def resize(value, width):
    if width >= value.size():
        return z3.ZeroExt(width - value.size(), value)
    return z3.Extract(width - 1, 0, value)


def limbs(value):
    return [
        z3.simplify(resize(
            z3.Extract(min(low + LIMB_BITS - 1, value.size() - 1), low, value),
            PRODUCT_BITS,
        ))
        for low in range(0, value.size(), LIMB_BITS)
    ]


def polynomial(a, b, width, multiply):
    """Shared by the actual rewrite and its universal ring certificate."""
    result = z3.BitVecVal(0, width)
    for i, x in enumerate(a):
        for j, y in enumerate(b):
            if (i + j) * LIMB_BITS < width:
                result += resize(multiply(x, y), width) * (1 << ((i + j) * LIMB_BITS))
    return result


def prove(formula):
    solver = z3.Solver()
    solver.set(timeout=30_000)
    solver.add(z3.Not(formula))
    result = solver.check()
    if result != z3.unsat:
        raise RuntimeError(f"rewrite certificate failed: {result}")


def certify(widths):
    for width in sorted(widths):
        if not 1 <= width <= 512:
            raise ValueError(f"unsupported multiplication width: {width}")
        value = z3.BitVec("certificate_value", width)
        reconstructed = sum(
            resize(limb, width) * (1 << (i * LIMB_BITS))
            for i, limb in enumerate(limbs(value))
        )
        prove(value == reconstructed)

        count = len(limbs(value))
        a = [z3.BitVec(f"certificate_a{i}", width) for i in range(count)]
        b = [z3.BitVec(f"certificate_b{i}", width) for i in range(count)]
        left = sum(x * (1 << (i * LIMB_BITS)) for i, x in enumerate(a))
        right = sum(y * (1 << (j * LIMB_BITS)) for j, y in enumerate(b))
        expanded = polynomial(a, b, width, lambda x, y: x * y)
        identity = z3.simplify(
            left * right == expanded, som=True, som_blowup=1000, mul2concat=False
        )
        if not z3.is_true(identity):
            raise RuntimeError("modular distributivity certificate failed")

        x, y = z3.BitVecs("certificate_x certificate_y", LIMB_BITS)
        # A 32-by-32 product is exact in u64. Extension/truncation preserves
        # multiplication modulo 2^width, including widths below 32.
        prove(
            resize(x, width) * resize(y, width)
            == resize(resize(x, PRODUCT_BITS) * resize(y, PRODUCT_BITS), width)
        )

    x, y = z3.BitVecs("commute_x commute_y", PRODUCT_BITS)
    prove(x * y == y * x)

    x, y = z3.BitVecs("bound_x bound_y", LIMB_BITS)
    product = resize(x, PRODUCT_BITS) * resize(y, PRODUCT_BITS)
    solver = z3.Solver()
    solver.add(z3.Not(z3.And(
        z3.ULE(product, PRODUCT_LIMIT),
        z3.Implies(z3.Or(x == 0, y == 0), product == 0),
    )))
    result = subprocess.run(
        [os.environ["KANI_CVC5"], "--lang=smt2", "--solve-bv-as-int=sum",
         "--ext-rew-prep=agg", "--tlimit=30000"],
        input="(set-logic ALL)\n" + solver.sexpr() + "\n(check-sat)\n",
        capture_output=True, text=True, timeout=35, check=True,
    )
    if result.stdout.strip() != "unsat":
        raise RuntimeError("partial-product abstraction certificate failed")


class ProductAbstraction:
    def __init__(self):
        self.products = {}
        self.widths = set()

    def partial(self, x, y):
        if z3.is_bv_value(x) or z3.is_bv_value(y):
            return z3.simplify(x * y)
        key = tuple(sorted(
            (z3.simplify(z3.Extract(LIMB_BITS - 1, 0, a)) for a in (x, y)),
            key=lambda a: a.sexpr(),
        ))
        if key not in self.products:
            self.products[key] = z3.FreshConst(z3.BitVecSort(PRODUCT_BITS))
        return self.products[key]

    @functools.lru_cache(None)
    def rewrite(self, expression):
        if z3.is_quantifier(expression):
            raise ValueError("only quantifier-free CBMC formulas are supported")
        if not z3.is_app(expression) or not expression.num_args():
            return expression
        args = [self.rewrite(a) for a in expression.children()]
        expression = expression.decl()(*args)
        if expression.decl().kind() == z3.Z3_OP_BMUL:
            if len(args) != 2:
                raise ValueError("expected binary bit-vector multiplication")
            width = expression.size()
            self.widths.add(width)
            expression = polynomial(limbs(args[0]), limbs(args[1]), width, self.partial)
        return z3.simplify(expression)

    def constraints(self):
        for (x, y), product in self.products.items():
            yield z3.ULE(product, PRODUCT_LIMIT)
            yield z3.Implies(z3.Or(x == 0, y == 0), product == 0)


def simplify_goal(formula):
    goal = z3.Goal()
    goal.add(formula)
    goals = z3.Then("simplify", "propagate-values", "solve-eqs", "simplify")(goal)
    if len(goals) != 1:
        raise RuntimeError("unexpected split during equisatisfiable preprocessing")
    return goals[0].as_expr()


def solve_formula(formula):
    formula = simplify_goal(formula)
    abstraction = ProductAbstraction()
    formula = simplify_goal(abstraction.rewrite(formula))
    certify(abstraction.widths)
    solver = z3.Solver()
    solver.set(timeout=240_000)
    solver.add(formula, *abstraction.constraints())
    # An abstract SAT result need not be a real counterexample. Never emit
    # a fabricated model; CBMC treats unknown as failed verification.
    return "unsat" if solver.check() == z3.unsat else "unknown"


if __name__ == "__main__":
    try:
        if len(sys.argv) != 3 or sys.argv[1] != "-smt2":
            raise ValueError("expected CBMC invocation: z3 -smt2 FILE")
        print(solve_formula(z3.And(*z3.parse_smt2_file(sys.argv[2]))))
    except Exception as error:
        print(f"word-product verification failed: {error}", file=sys.stderr)
        print("unknown")
        sys.exit(1)
