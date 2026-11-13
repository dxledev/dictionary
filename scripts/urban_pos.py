#!/usr/bin/env python3
"""Conservative Urban Dictionary part-of-speech fallback."""

from __future__ import annotations

import json
import os
import sys
from typing import Any


MAX_REQUEST_BYTES = 65536
MAX_ITEMS = 64
MAX_WORD_CHARS = 128
MAX_EXAMPLE_CHARS = 4096
MAX_TOTAL_EXAMPLE_CHARS = 16384
MAX_SENTENCE_TOKENS = 256
MAX_TOTAL_TOKENS = 4096
DEFAULT_MIN_CONFIDENCE = 0.9

SUPPORTED_TAGS = {
    "NN": "noun", "NNS": "noun", "NNP": "noun", "NNPS": "noun",
    "VB": "verb", "VBD": "verb", "VBG": "verb", "VBN": "verb",
    "VBP": "verb", "VBZ": "verb", "MD": "verb",
    "JJ": "adjective", "JJR": "adjective", "JJS": "adjective",
    "RB": "adverb", "RBR": "adverb", "RBS": "adverb",
    "UH": "interjection",
}


def _single_token_word(word: Any) -> bool:
    return (
        isinstance(word, str)
        and bool(word)
        and len(word) <= MAX_WORD_CHARS
        and not any(char.isspace() for char in word)
    )


def _confidence_limit() -> float:
    try:
        value = float(os.environ.get("DICTIONARY_NLTK_MIN_CONFIDENCE", DEFAULT_MIN_CONFIDENCE))
    except (TypeError, ValueError):
        return DEFAULT_MIN_CONFIDENCE
    if not 0.0 <= value <= 1.0:
        return DEFAULT_MIN_CONFIDENCE
    return value


def _sentence_token_spans(sentence_tokenizer: Any, text: str) -> list[tuple[int, int]]:
    return list(sentence_tokenizer.span_tokenize(text))


def _token_spans(tokenizer: Any, text: str) -> list[tuple[int, int]]:
    return list(tokenizer.span_tokenize(text))


def _tag_sentence(
    sentence: str,
    word: str,
    tokenizer: Any,
    tagger: Any,
    min_confidence: float,
) -> tuple[list[str], int]:
    spans = _token_spans(tokenizer, sentence)
    if len(spans) > MAX_SENTENCE_TOKENS:
        raise ValueError("sentence token limit exceeded")
    tokens = [sentence[start:end] for start, end in spans]
    positions = [index for index, token in enumerate(tokens) if token.casefold() == word.casefold()]
    if not positions:
        return [], len(tokens)

    tagdict = getattr(tagger, "tagdict", None)
    removed: dict[str, str] = {}
    if isinstance(tagdict, dict):
        for key in list(tagdict):
            if isinstance(key, str) and key.casefold() == word.casefold():
                removed[key] = tagdict.pop(key)
    try:
        tagged = tagger.tag(tokens, return_conf=True, use_tagdict=True)
    finally:
        if isinstance(tagdict, dict):
            tagdict.update(removed)
    if len(tagged) != len(tokens):
        return [""] * len(positions), len(tokens)

    results: list[str] = []
    for index in positions:
        item = tagged[index]
        if len(item) != 3:
            results.append("")
            continue
        _, tag, confidence = item
        category = SUPPORTED_TAGS.get(tag)
        if not category or float(confidence) < min_confidence:
            results.append("")
        else:
            results.append(category)
    return results, len(tokens)


def _is_metalinguistic(sentence: str, word: str) -> bool:
    import re

    quoted_word = re.escape(word)
    patterns = (
        rf"\b{quoted_word}\b\s+(?:is\s+)?(?:a\s+)?word\b",
        rf"\b{quoted_word}\b\s+means\b",
        rf"\b{quoted_word}\b\s+is\s+slang\s+for\b",
    )
    return any(re.search(pattern, sentence, flags=re.IGNORECASE) for pattern in patterns)


def classify_example(
    word: Any,
    example: Any,
    tokenizer: Any,
    sentence_tokenizer: Any,
    tagger: Any,
    min_confidence: float = DEFAULT_MIN_CONFIDENCE,
) -> str | None:
    """Return a POS only when every tokenized headword occurrence agrees."""
    if not _single_token_word(word) or not isinstance(example, str):
        return None
    if len(example) > MAX_EXAMPLE_CHARS:
        return None

    categories: list[str] = []
    total_tokens = 0
    for start, end in _sentence_token_spans(sentence_tokenizer, example):
        sentence = example[start:end]
        if _is_metalinguistic(sentence, word):
            continue
        occurrence_results, token_count = _tag_sentence(
            sentence, word, tokenizer, tagger, min_confidence
        )
        total_tokens += token_count
        if total_tokens > MAX_TOTAL_TOKENS:
            return None
        categories.extend(occurrence_results)

    if not categories or any(not category for category in categories):
        return None
    if any(category != categories[0] for category in categories[1:]):
        return None
    return categories[0]


def classify_items(
    items: Any,
    tokenizer: Any,
    sentence_tokenizer: Any,
    tagger: Any,
    min_confidence: float = DEFAULT_MIN_CONFIDENCE,
) -> list[str | None]:
    if not isinstance(items, list) or len(items) > MAX_ITEMS:
        return []
    output: list[str | None] = []
    total_example_chars = 0
    for item in items:
        if not isinstance(item, dict):
            output.append(None)
            continue
        example = item.get("example")
        if isinstance(example, str):
            total_example_chars += min(len(example), MAX_EXAMPLE_CHARS + 1)
        if total_example_chars > MAX_TOTAL_EXAMPLE_CHARS:
            output.append(None)
            continue
        try:
            output.append(classify_example(
                item.get("word"), item.get("example"), tokenizer,
                sentence_tokenizer, tagger, min_confidence,
            ))
        except Exception:
            output.append(None)
    return output


def _load_nltk() -> tuple[Any, Any, Any]:
    from nltk.tokenize import PunktSentenceTokenizer, TreebankWordTokenizer
    from nltk.tag import PerceptronTagger

    return TreebankWordTokenizer(), PunktSentenceTokenizer(), PerceptronTagger(lang="eng")


def main() -> int:
    raw = sys.stdin.buffer.read(MAX_REQUEST_BYTES + 1)
    if len(raw) > MAX_REQUEST_BYTES:
        print(json.dumps([]))
        return 0
    try:
        items = json.loads(raw)
    except (json.JSONDecodeError, UnicodeDecodeError):
        print(json.dumps([]))
        return 0
    if not isinstance(items, list) or len(items) > MAX_ITEMS:
        print(json.dumps([]))
        return 0
    try:
        tokenizer, sentence_tokenizer, tagger = _load_nltk()
    except Exception as error:
        print(f"urban_pos: NLTK unavailable: {error}", file=sys.stderr)
        print(json.dumps([None] * len(items)))
        return 0
    results = classify_items(items, tokenizer, sentence_tokenizer, tagger, _confidence_limit())
    print(json.dumps(results, separators=(",", ":")))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
