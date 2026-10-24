#!/usr/bin/env python3
"""Cheap observer controls; no compositor, Pillow, or product process required."""
import copy
import json
import unittest

import fileio_live as fio

DOC = {"x": 0, "y": 0, "width": 1600, "height": 954}
ECHO = {"x": 0, "y": 977, "width": 1600, "height": 23}


def glyphs(text, clip, y=0):
    return [{"role": "Text", "clip": clip, "x": x * 12, "y": y, "char": char}
            for x, char in enumerate(text)]


def scene(*parts):
    return json.dumps({"diagnostic": "submitted-scene", "glyphs": sum(parts, [])}) + "\n"


class DocumentObserver(unittest.TestCase):
    def test_save_echo_does_not_change_document(self):
        before = scene(glyphs("* Heading λ!", DOC))
        after = scene(glyphs("* Heading λ!", DOC), glyphs("Saved /private/notes.org", ECHO, 977))
        self.assertEqual(fio.document_rows(before, DOC), fio.document_rows(after, DOC))
        self.assertIn("Saved /private/notes.org", fio.rows(after))

    def test_document_corruption_still_fails_equality(self):
        before = scene(glyphs("* Heading λ!", DOC))
        after = scene(glyphs("* Heading CORRUPTED", DOC), glyphs("Saved", ECHO, 977))
        with self.assertRaises(AssertionError):
            self.assertEqual(fio.document_rows(before, DOC), fio.document_rows(after, DOC))

    def test_saved_is_legitimate_document_text(self):
        text = scene(glyphs("Saved", DOC), glyphs("Saved", ECHO, 977))
        self.assertEqual(fio.document_rows(text, DOC), ["Saved"])

    def test_exact_clip_not_just_y_or_clip_key_order(self):
        neighbour = {"height": 954, "width": 800, "y": 0, "x": 800}
        reordered = {"height": 954, "width": 1600, "y": 0, "x": 0}
        text = scene(glyphs("document", reordered), glyphs("other window", neighbour))
        self.assertEqual(fio.document_rows(text, DOC), ["document"])

    def test_all_rows_keeps_close_cancel_and_supersession_prompts(self):
        for prompt in ("Discard? (y/n; C-g cancels)", "Cancelled", "File changed on disk"):
            text = scene(glyphs("document", DOC), glyphs(prompt, ECHO, 977))
            self.assertIn(prompt, fio.rows(text))
            self.assertEqual(fio.document_rows(text, DOC), ["document"])

    def test_full_document_not_first_six_rows_and_missing_clip_refuses(self):
        before = scene(*(glyphs(str(i), DOC, i * 23) for i in range(8)))
        after = scene(*(glyphs(str(i) if i != 7 else "CORRUPTED", DOC, i * 23) for i in range(8)))
        self.assertEqual(len(fio.document_rows(before, DOC)), 8)
        self.assertNotEqual(fio.document_rows(before, DOC), fio.document_rows(after, DOC))
        with self.assertRaises(AssertionError):
            fio.document_rows(scene(glyphs("Saved", ECHO, 977)), DOC)

    def test_last_complete_jsonl_scene_only(self):
        text = scene(glyphs("stale", DOC)) + "renderer log\n" + scene(glyphs("current", DOC))
        text += '{"diagnostic":"submitted-scene","glyphs":['
        self.assertEqual(fio.document_rows(text, DOC), ["current"])

    def test_pin_requires_one_exact_document_rowset(self):
        text = scene(glyphs("document", DOC), glyphs("Saved", ECHO, 977))
        self.assertEqual(fio.document_clip(text, ["document"]), DOC)
        with self.assertRaises(AssertionError):
            fio.document_clip(text, ["absent"])
        duplicate = dict(DOC, x=800)
        with self.assertRaises(AssertionError):
            fio.document_clip(scene(glyphs("document", DOC), glyphs("document", duplicate)), ["document"])

    def test_jsonl_unicode_separators_remain_inside_record(self):
        text = "NEL\u0085LINE\u2028PARAGRAPH\u2029"
        record = json.dumps({"diagnostic": "submitted-scene", "glyphs": glyphs(text, DOC)},
                            ensure_ascii=False) + "\n"
        self.assertEqual(fio.document_rows(record, DOC), [text])

    def test_recorded_sequence_preserves_document_and_rejects_mutation(self):
        text = (scene(glyphs("* Heading λ!", DOC))
                + scene(glyphs("* Heading λ!", DOC), glyphs("Saved", ECHO, 977)))
        scenes = [json.loads(line) for line in text.split('\n')
                  if line.startswith("{") and '"submitted-scene"' in line]
        modified = next(s for s in scenes if "* Heading λ!" in fio.rows(json.dumps(s)))
        saved = scenes[-1]
        before, after = json.dumps(modified), json.dumps(saved)
        self.assertNotEqual(fio.rows(before), fio.rows(after))
        self.assertEqual(fio.document_rows(before, DOC), fio.document_rows(after, DOC))
        corrupt = copy.deepcopy(saved)
        next(g for g in corrupt["glyphs"] if g["role"] == "Text" and g["clip"] == DOC)["char"] = "!"
        with self.assertRaises(AssertionError):
            self.assertEqual(fio.document_rows(before, DOC), fio.document_rows(json.dumps(corrupt), DOC))


if __name__ == "__main__":
    unittest.main()
