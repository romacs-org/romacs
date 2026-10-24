import json
import unittest

from find_file_live import current_rows


def document(text):
    return json.dumps({"diagnostic": "submitted-scene", "glyphs": [
        {"role": "Text", "clip": [0, 0, 100, 20], "x": i, "y": 0, "char": char}
        for i, char in enumerate(text)
    ]}) + "\n"


def prompt(text):
    return json.dumps({"diagnostic": "prompt-paint", "minibuffer": text}) + "\n"


class CurrentRows(unittest.TestCase):
    def test_prompt_supersedes_the_previous_document(self):
        self.assertEqual(current_rows(document("old") + prompt("Find file: new")), ["Find file: new"])

    def test_returned_document_supersedes_stale_prompt(self):
        self.assertEqual(current_rows(prompt("Switch to buffer: A") + document("B")), ["B"])

    def test_partial_record_cannot_discard_last_complete_frame(self):
        self.assertEqual(current_rows(document("kept") + '{"diagnostic":"prompt-paint",'), ["kept"])

    def test_unicode_line_separator_is_not_a_log_boundary(self):
        text = "Find file: λ\u2028name"
        record = json.dumps({"diagnostic": "prompt-paint", "minibuffer": text}, ensure_ascii=False)
        self.assertEqual(current_rows(document("old") + record + "\n"), [text])


if __name__ == "__main__":
    unittest.main()
