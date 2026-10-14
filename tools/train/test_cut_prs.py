#!/usr/bin/env python3
"""Offline unit tests for cut_prs.py: recorded gh responses, no network.

Run: python3 tools/train/test_cut_prs.py
"""
import json
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import cut_prs  # noqa: E402

FIX = os.path.join(os.path.dirname(os.path.abspath(__file__)), "fixtures")


def cut(tags, version):
    return version, tags


def pull(n, base="main", merged=True):
    return {"number": n, "base": {"ref": base}, "merged_at": "2026-10-13T00:00:00Z" if merged else None}


class FakeGh:
    """Replays recorded responses keyed by path prefix; unknown paths raise."""

    def __init__(self, compares, pulls):
        self.compares, self.pulls, self.calls = compares, pulls, []

    def __call__(self, path):
        self.calls.append(path)
        if "/compare/" in path:
            key = path.split("/compare/")[1].split("?")[0]
            repo = path.split("/")[2]
            if (repo, key) not in self.compares:
                raise RuntimeError(f"gh api {path}: Not Found (HTTP 404)")
            shas = self.compares[(repo, key)]
            page = int(path.split("&page=")[1])
            return {"total_commits": len(shas),
                    "commits": [{"sha": s} for s in shas[(page - 1) * 100:page * 100]]}
        if "/commits/" in path:
            sha = path.split("/commits/")[1].split("/")[0]
            return self.pulls[sha]
        raise AssertionError(path)


class CutPrsTest(unittest.TestCase):
    def test_recorded_fixture(self):
        with open(os.path.join(FIX, "compare_cli.json")) as f:
            rec = json.load(f)
        gh = FakeGh(
            {("enscrive-cli", "v1...v2"): rec["shas"]},
            rec["pulls"],
        )
        out = cut_prs.build(cut({"enscrive": "v1"}, "P"), cut({"enscrive": "v2"}, "C"), None, gh)
        self.assertEqual(out["errors"], [])
        self.assertEqual(out["cut_prs"], rec["expect"])

    def test_unchanged_tag_contributes_nothing(self):
        gh = FakeGh({}, {})
        out = cut_prs.build(cut({"esm": "v1"}, "P"), cut({"esm": "v1"}, "C"), None, gh)
        self.assertEqual(out["components"], {})
        self.assertEqual(out["cut_prs"], [])
        self.assertEqual(gh.calls, [])

    def test_changed_tag_and_dedup_and_filters(self):
        gh = FakeGh(
            {("enscrive-deploy", "a...b"): ["s1", "s2", "s3", "s4"]},
            {
                "s1": [pull(7)],
                "s2": [pull(7), pull(5)],  # same PR twice -> dedup
                "s3": [pull(9, base="feature"), pull(11, merged=False)],  # filtered out
                "s4": [],  # direct push
            },
        )
        out = cut_prs.build(cut({}, "P"), cut({}, "C"), ("a", "b"), gh)
        self.assertEqual(out["cut_prs"], ["enscrive-deploy#5", "enscrive-deploy#7"])
        self.assertEqual(out["components"]["enscrive-deploy"]["unattributed"], ["s3", "s4"])

    def test_missing_tag_is_error(self):
        out = cut_prs.build(cut({"esm": "v1"}, "P"), cut({"esm": "vMISSING"}, "C"), None, FakeGh({}, {}))
        self.assertEqual(len(out["errors"]), 1)
        self.assertIn("esm", out["errors"][0])
        self.assertEqual(out["components"], {})

    def test_main_exits_nonzero_and_empties_list_on_error(self):
        import io
        import tempfile
        import contextlib

        with tempfile.TemporaryDirectory() as d:
            paths = []
            for name, v in (("p.json", "v1"), ("c.json", "vX")):
                p = os.path.join(d, name)
                with open(p, "w") as f:
                    json.dump({"version": "V" + v, "binaries": {"esm": {"source_version": v}}}, f)
                paths.append(p)
            buf, err = io.StringIO(), io.StringIO()
            with contextlib.redirect_stdout(buf), contextlib.redirect_stderr(err):
                rc = cut_prs.main(["--prev", paths[0], "--cur", paths[1], "--json"], gh=FakeGh({}, {}))
        self.assertEqual(rc, 1)
        self.assertEqual(json.loads(buf.getvalue())["cut_prs"], [])

    def test_grammar(self):
        ok = ["enscrive-cli#1", "a#123456789", "x" * 64 + "#1"]
        self.assertEqual(cut_prs.validate_cut_prs(ok), [])
        bad = ["Enscrive#1", "enscrive/cli#1", "cli#", "cli#1234567890", "cli#1\n",
               "#1", "x" * 65 + "#1", "cli_x#1", "cli#-1", " cli#1"]
        for b in bad:
            self.assertEqual(len(cut_prs.validate_cut_prs([b])), 1, b)

    def test_cap(self):
        at_cap = [f"r#{i}" for i in range(1, 1025)]
        self.assertEqual(cut_prs.validate_cut_prs(at_cap), [])
        errs = cut_prs.validate_cut_prs(at_cap + ["r#1025"])
        self.assertEqual(len(errs), 1)
        self.assertIn("cap", errs[0])

    def test_over_cap_build_errors(self):
        shas = [f"s{i}" for i in range(1025)]
        gh = FakeGh({("enscrive-cli", "v1...v2"): shas}, {s: [pull(i + 1)] for i, s in enumerate(shas)})
        out = cut_prs.build(cut({"enscrive": "v1"}, "P"), cut({"enscrive": "v2"}, "C"), None, gh)
        self.assertTrue(any("cap" in e for e in out["errors"]))

    def test_pinset_toml_alias(self):
        import tempfile

        with tempfile.TemporaryDirectory() as d:
            p = os.path.join(d, "dev.toml")
            with open(p, "w") as f:
                f.write('platform_version = "PV"\n[overrides]\nenscrive-cli = "v9"\n')
            ver, tags = cut_prs.load_cut(p)
        self.assertEqual((ver, tags), ("PV", {"enscrive": "v9"}))


if __name__ == "__main__":
    unittest.main()
