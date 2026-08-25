# Desktop Translator — English notes

The primary project documentation is in [README.md](README.md). This page
records the English privacy contract for the local vocabulary feature.

## Privacy and local learning data

Desktop Translator contacts the configured translation provider only after an
explicit Translate action. Eligible words and short lexical phrases, successful
translation payloads, lookup counts, and submitted practice outcomes are stored
in an application-local SQLite database. Sentence-like text is not added to the
wordbook.

The database remains inside the native application, is not exposed to the
WebView, and is not synchronized to a cloud service. A repeated lookup increases
lookup demand but does not claim that the word was remembered. Recall state
changes only after the user explicitly submits a practice answer.

Downloaded textbook files and related-word discovery remain local. Connections
combine conservative root matching with shared meanings from the personal
wordbook and compatible downloaded textbooks. No embeddings or LLM analysis is
involved. Textbook attribution and source links remain visible in the study UI.

Translation lookup follows this order: personal wordbook, active downloaded
textbook, then the configured online translation provider. A textbook hit is
promoted into the personal wordbook for later study.

Windows 10/11 x64 is distributed as an unsigned NSIS installer. Compile, gating
CI, and silent install/launch/uninstall were recorded on Windows 11 25H2 x64;
interactive overlay and speech fixtures remain manual-only. See
[README.md](README.md) and
[docs/windows-signing-and-supply-chain.md](docs/windows-signing-and-supply-chain.md).
