#!/usr/bin/env python3
"""Self-test of validate.py: python3 scripts/test_validate.py"""
import copy
import json
import os
import tempfile
import unittest

import validate

GOOD = json.load(open(os.path.join(os.path.dirname(__file__), "..", "servers.example.json")))["servers"][0]


def problems(entry_changes=None, extra=None):
    e = copy.deepcopy(GOOD)
    for k, v in (entry_changes or {}).items():
        if v is None:
            e.pop(k, None)
        else:
            e[k] = v
    servers = [e] + (extra or [])
    with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as f:
        json.dump({"schema": 1, "servers": servers}, f)
    try:
        return validate.check_file(f.name)
    finally:
        os.unlink(f.name)


class Validate(unittest.TestCase):
    def test_the_example_is_valid(self):
        self.assertEqual(problems(), [])

    def test_empty_list_is_valid(self):
        self.assertEqual(validate.check_file(os.path.join(os.path.dirname(__file__), "..", "servers.json")), [])

    def test_required_fields(self):
        for k in validate.REQUIRED:
            self.assertTrue(problems({k: None}), k)

    def test_ids(self):
        for bad in ["Ab", "UPPER", "-lead", "trail-", "has space", "x" * 49, "ab"]:
            self.assertTrue(problems({"id": bad}), bad)
        self.assertFalse(problems({"id": "abc"}))

    def test_local_addresses_are_refused(self):
        for bad in ["http://127.0.0.1:8080", "192.168.1.5", "localhost:8080", "https://10.0.0.1", "box.local", "http://[::1]:8080", "singlelabel"]:
            self.assertTrue(problems({"address": bad}), bad)
        self.assertFalse(problems({"address": "http://93.184.216.34:8080"}))
        self.assertFalse(problems({"address": "play.example.org:8443"}))

    def test_address_is_a_base_address(self):
        self.assertTrue(problems({"address": "https://play.example.org/some/path"}))
        self.assertTrue(problems({"address": "https://play.example.org/?x=1"}))
        self.assertFalse(problems({"address": "https://play.example.org/api/v1"}))

    def test_duplicates(self):
        other = dict(GOOD, name="Other")
        self.assertTrue(problems(extra=[other]))  # same id and address
        self.assertTrue(problems(extra=[dict(other, id="other-realm")]))  # same address
        self.assertFalse(problems(extra=[dict(other, id="other-realm", address="https://other.example.org")]))

    def test_tags_mods_and_unknown_keys(self):
        self.assertTrue(problems({"tags": ["UPPER"]}))
        self.assertTrue(problems({"tags": ["a", "a"]}))
        self.assertTrue(problems({"tags": ["t%d" % i for i in range(9)]}))
        self.assertTrue(problems({"mods": [{"id": "x", "url": "http://x.org/a.zip"}]}))
        self.assertTrue(problems({"mods": [{"id": "has space"}]}))
        self.assertTrue(problems({"colour": "red"}))
        self.assertTrue(problems({"description": "x" * 401}))

    def test_bad_files(self):
        with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as f:
            f.write("{not json")
        self.assertTrue(validate.check_file(f.name))
        os.unlink(f.name)
        with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as f:
            json.dump({"schema": 2, "servers": []}, f)
        self.assertTrue(validate.check_file(f.name))
        os.unlink(f.name)


if __name__ == "__main__":
    unittest.main()
