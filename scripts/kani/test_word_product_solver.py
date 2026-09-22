"""Tests for the conservative SMT adapter, including deliberate proof faults."""

import os
from pathlib import Path
import subprocess
import sys
import unittest
from unittest.mock import patch

import z3

import word_product_solver as proof


class WordProductSolverTest(unittest.TestCase):
    def test_rule_certificates_cover_partial_and_wide_words(self):
        proof.certify({1, 31, 32, 33, 64, 65, 128, 129, 256})

    def test_reconstruction_certificate_rejects_missing_limb(self):
        original = proof.limbs
        with patch.object(proof, "limbs", lambda value: original(value)[1:]):
            with self.assertRaises(RuntimeError):
                proof.certify({64})

    def test_distribution_certificate_rejects_missing_carry(self):
        original = proof.polynomial
        with patch.object(proof, "polynomial", lambda *args: original(*args) + 1):
            with self.assertRaises(RuntimeError):
                proof.certify({64})

    def test_partial_product_certificate_rejects_unknown(self):
        unknown = subprocess.CompletedProcess([], 0, "unknown\n", "")
        with patch.object(proof.subprocess, "run", return_value=unknown):
            with self.assertRaises(RuntimeError):
                proof.certify(set())

    def test_partial_product_certificate_rejects_timeout(self):
        with patch.object(proof.subprocess, "run", side_effect=subprocess.TimeoutExpired([], 35)):
            with self.assertRaises(subprocess.TimeoutExpired):
                proof.certify(set())

    def test_proves_equivalence_and_rejects_wrong_result(self):
        x, y = z3.BitVecs("test_x test_y", 64)
        product = z3.ZeroExt(64, x) * z3.ZeroExt(64, y)
        # Native low-word multiplication must match the low half of u128.
        self.assertEqual(proof.solve_formula(x * y != z3.Extract(63, 0, product)), "unsat")
        # Adding one is wrong. A real or abstract counterexample must fail.
        self.assertEqual(proof.solve_formula(x * y + 1 != z3.Extract(63, 0, product)), "unknown")

    def test_constant_products_and_zero_constraint(self):
        x, y = z3.BitVecs("zero_x zero_y", 64)
        self.assertEqual(proof.solve_formula(z3.And(x == 0, x * y != 0)), "unsat")
        self.assertEqual(proof.solve_formula(x * 5 != x + x + x + x + x), "unsat")

    def test_unsupported_input_fails_closed(self):
        x = z3.BitVec("quantified_x", 64)
        with self.assertRaises(ValueError):
            proof.ProductAbstraction().rewrite(z3.ForAll([x], x * x == x))
        with self.assertRaises(ValueError):
            proof.certify({513})

    def test_bad_invocation_and_missing_input_fail_closed(self):
        script = Path(proof.__file__)
        for args in [[], ["-smt2", "/nonexistent/manifest-arithmetic-proof.smt2"]]:
            result = subprocess.run([sys.executable, str(script), *args], capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(result.stdout.strip(), "unknown")


if __name__ == "__main__":
    if not os.environ.get("KANI_CVC5"):
        raise SystemExit("Set KANI_CVC5 to the pinned cvc5 binary before testing")
    unittest.main()
