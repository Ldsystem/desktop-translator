# Desktop Translator — English guide

The complete, maintained English documentation is [README.md](README.md). For Chinese, see [README.zh-CN.md](README.zh-CN.md).

The guide describes the **0.6.0 source**; check the selected release tag when downloading. It covers selection and Quick Translate, multiple saved meanings with POS, card-to-detail navigation, verified textbook connections, and the downloaded textbook shelf.

| Looking for | Read |
| --- | --- |
| Installation and provider setup | [Install and start](README.md#install-and-start) |
| Vocabulary cards, details, related words, and practice | [Vocabulary Study](README.md#vocabulary-study) |
| Dictionary coverage and connection-count rules | [Where extra meanings and connections come from](README.md#where-extra-meanings-and-connections-come-from) |
| Downloaded books and refresh actions | [Textbook shelf](README.md#textbook-shelf) |
| Local storage, accessibility, credentials, and network behavior | [Privacy and data](README.md#privacy-and-data) |
| Prerequisites, checks, packaging, and source layout | [Development](README.md#development) |

## Current app screenshot

Captured from the installed pre-release macOS build on **2026-09-03**, with the user's Simplified Chinese interface retained. The [main guide](README.md#vocabulary-study) also includes fresh word-detail, related-word, and downloaded-textbook screenshots.

![Current personal wordbook in the installed macOS app](docs/screenshots/vocabulary-study-wordbook.jpg)

## Privacy at a glance

Learning records stay in local SQLite storage, without built-in cloud sync. A repeated lookup increases lookup demand; recall changes after a submitted practice answer. Credentials stay in native system-vault storage, and the web interface receives status rather than secret values.

Selection access may read surrounding accessibility text to record a local example; it does not use screenshots, OCR, or an automatic clipboard fallback. Translation requests send the requested text and language direction to the selected provider, not the saved example. Explicit enrichment, provider checks, language retrieval, and textbook downloads can also use the network.

Related words use verified local lexical data and compatible installed textbooks, not embeddings or an LLM. “All meanings” refers to available saved senses, not complete dictionary coverage for every word.
