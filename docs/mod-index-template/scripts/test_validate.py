#!/usr/bin/env python3
"""Tests of validate.py: python3 scripts/test_validate.py"""
import copy
import json
import os
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import validate  # noqa: E402

ROOT = os.path.dirname(HERE)


def good():
    return {
        "schema": 1,
        "updated": "2026-10-22T00:00:00Z",
        "mods": [
            {
                "id": "hud-tweaks", "name": "HUD tweaks", "description": "d", "author": "Ann", "side": "client",
                "tags": ["ui"], "homepage": "https://example.org/x", "licence": "MIT",
                "versions": [
                    {"version": "1.0.0", "game_versions": ["0.5.5e.hotfix1"], "hook_api": 1, "sha256": "a" * 64,
                     "size": 10, "url": "https://example.org/hud-tweaks-1.0.0.zip"}
                ],
            }
        ],
    }


def problems(d):
    return validate.check_data(d)


class Validate(unittest.TestCase):
    def test_a_good_index_passes(self):
        self.assertEqual(problems(good()), [])

    def test_the_shipped_files_are_valid(self):
        for name in ("index.json", "index.example.json"):
            p, _ = validate.check_file(os.path.join(ROOT, name))
            self.assertEqual(p, [], name)

    def test_the_empty_template_is_valid(self):
        self.assertEqual(problems({"schema": 1, "mods": []}), [])

    def test_top_level(self):
        d = good(); d["schema"] = 2
        self.assertTrue(any("schema" in x for x in problems(d)))
        d = good(); d["extra"] = 1
        self.assertTrue(any("unknown key" in x for x in problems(d)))
        self.assertTrue(problems([]))
        d = good(); d["mods"] = {}
        self.assertTrue(problems(d))

    def test_mod_fields(self):
        cases = [
            ("id", "Bad Id"), ("id", "x"), ("name", ""), ("side", "weird"), ("tags", ["Bad Tag"]),
            ("tags", ["a", "a"]), ("homepage", "http://example.org"), ("homepage", "javascript:alert(1)"),
            ("versions", []), ("description", "x" * 601), ("licence", 5),
        ]
        for key, value in cases:
            d = good(); d["mods"][0][key] = value
            self.assertTrue(problems(d), f"{key}={value!r} must be refused")
        d = good(); d["mods"][0]["bogus"] = 1
        self.assertTrue(any("unknown key" in x for x in problems(d)))

    def test_version_fields(self):
        cases = [
            ("version", "1.0"), ("version", "v1.0.0"), ("sha256", "zz"), ("sha256", "a" * 63), ("size", 0),
            ("size", 600 << 20), ("size", True), ("url", "http://example.org/a.zip"),
            ("url", "https://user:pw@example.org/a.zip"), ("url", "https://example.org/a b.zip"),
            ("hook_api", -1), ("hook_api", "1"), ("game_versions", "0.5.5e.hotfix1"), ("yanked", "yes"),
        ]
        for key, value in cases:
            d = good(); d["mods"][0]["versions"][0][key] = value
            self.assertTrue(problems(d), f"{key}={value!r} must be refused")
        d = good(); d["mods"][0]["versions"][0]["nope"] = 1
        self.assertTrue(any("unknown key" in x for x in problems(d)))

    def test_duplicates(self):
        d = good(); d["mods"].append(copy.deepcopy(d["mods"][0]))
        self.assertTrue(any("duplicate id" in x for x in problems(d)))
        d = good(); d["mods"][0]["versions"].append(copy.deepcopy(d["mods"][0]["versions"][0]))
        self.assertTrue(any("twice" in x for x in problems(d)))

    def test_every_problem_is_listed(self):
        d = good(); d["mods"][0]["side"] = "x"; d["mods"][0]["versions"][0]["sha256"] = "q"
        self.assertGreaterEqual(len(problems(d)), 2)

    def test_optional_values_may_be_null_or_missing(self):
        d = good(); v = d["mods"][0]["versions"][0]
        v["server_api"] = None; v["yanked"] = True; v["released"] = "2026-10-20"
        d["mods"][0]["homepage"] = ""
        self.assertEqual(problems(d), [])
        del v["hook_api"], v["game_versions"]
        self.assertEqual(problems(d), [])

    def test_files_on_disk(self):
        import tempfile
        with tempfile.TemporaryDirectory() as t:
            p = os.path.join(t, "i.json")
            with open(p, "w") as f:
                f.write("{not json")
            self.assertEqual(validate.main([p]), 1)
            with open(p, "w") as f:
                json.dump(good(), f)
            self.assertEqual(validate.main([p]), 0)


if __name__ == "__main__":
    unittest.main()
