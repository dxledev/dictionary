import re
import unittest

from urban_pos import classify_example, classify_items


class WordTokenizer:
    def span_tokenize(self, text):
        return [(match.start(), match.end()) for match in re.finditer(r"\w+|[^\w\s]", text)]


class SentenceTokenizer:
    def span_tokenize(self, text):
        return [(match.start(), match.end()) for match in re.finditer(r"[^.!?]+(?:[.!?]+|$)", text)]


class FakeTagger:
    def __init__(self, target_tags):
        self.target_tags = iter(target_tags)
        self.calls = 0
        self.tagdict = {"YeEt": "NN"}
        self.tagdict_during_tag = None
        self.raise_on_tag = False

    def tag(self, tokens, return_conf=False, use_tagdict=True):
        self.calls += 1
        self.tagdict_during_tag = dict(self.tagdict)
        if self.raise_on_tag:
            raise RuntimeError("fake tagger failure")
        result = []
        for token in tokens:
            tag, confidence = next(self.target_tags, ("NN", 0.99)) if token.casefold() == "yeet" else ("NN", 0.99)
            result.append((token, tag, confidence))
        return result


class UrbanPosTests(unittest.TestCase):
    def classify(self, word, example, tags):
        return classify_example(
            word, example, WordTokenizer(), SentenceTokenizer(), FakeTagger(tags), 0.9
        )

    def test_classifies_example_occurrence(self):
        self.assertEqual(self.classify("yeet", "We yeet the ball!", [("VB", 0.99)]), "verb")

    def test_case_insensitive_and_all_occurrences_must_agree(self):
        self.assertEqual(
            self.classify("Yeet", "We yeet, then YEET again.", [("VB", 0.99), ("VB", 0.98)]),
            "verb",
        )
        self.assertIsNone(
            self.classify("yeet", "We yeet, then YEET again.", [("VB", 0.99), ("NN", 0.99)])
        )

    def test_requires_occurrence_in_example(self):
        tagger = FakeTagger([])
        self.assertIsNone(classify_example(
            "yeet", "A person throws the ball.", WordTokenizer(), SentenceTokenizer(), tagger
        ))
        self.assertEqual(tagger.calls, 0)

    def test_does_not_match_substrings(self):
        self.assertIsNone(self.classify("cap", "That is a capital idea.", [("NN", 0.99)]))

    def test_rejects_low_confidence_and_unsupported_tags(self):
        self.assertIsNone(self.classify("yeet", "We yeet.", [("VB", 0.89)]))
        self.assertIsNone(self.classify("yeet", "We yeet.", [("IN", 0.99)]))

    def test_skips_metalinguistic_examples(self):
        self.assertIsNone(self.classify("yeet", "Yeet means to throw something.", []))
        self.assertIsNone(self.classify("yeet", "Yeet is slang for throwing something.", []))

    def test_target_tagdict_entry_is_removed_only_during_tagging_and_restored(self):
        tagger = FakeTagger([("VB", 0.99)])
        result = classify_example("yeet", "We yeet.", WordTokenizer(), SentenceTokenizer(), tagger)
        self.assertEqual(result, "verb")
        self.assertEqual(tagger.tagdict_during_tag, {})
        self.assertEqual(tagger.tagdict, {"YeEt": "NN"})

        tagger.raise_on_tag = True
        with self.assertRaises(RuntimeError):
            classify_example("yeet", "We yeet.", WordTokenizer(), SentenceTokenizer(), tagger)
        self.assertEqual(tagger.tagdict, {"YeEt": "NN"})

    def test_multiword_and_invalid_items_abstain(self):
        result = classify_items(
            [
                {"word": "yeet on", "example": "We yeet on them."},
                {"word": "yeet", "example": None},
                None,
            ],
            WordTokenizer(), SentenceTokenizer(), FakeTagger([]),
        )
        self.assertEqual(result, [None, None, None])


if __name__ == "__main__":
    unittest.main()
