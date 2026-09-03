<div align="center">

<img src="src-tauri/icons/icon.png" alt="Desktop Translator icon" width="96" />

# Desktop Translator

**Translate in place. Build your wordbook.**

[![CI](https://github.com/Ldsystem/desktop-translator/actions/workflows/ci.yml/badge.svg)](https://github.com/Ldsystem/desktop-translator/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/Ldsystem/desktop-translator)](https://github.com/Ldsystem/desktop-translator/releases/latest)
[![Tauri 2](https://img.shields.io/badge/Tauri-2-24C8DB)](https://tauri.app)

English · [简体中文](README.zh-CN.md)

</div>

A lightweight macOS and Windows translator with an on-device vocabulary study window. Select text in a supported application and click the nearby translation button, or open Quick Translate from the menu bar / system tray to type or paste text.

Eligible word lookups build a personal wordbook. Review saved meanings and parts of speech, explore textbook connections, and practise without moving your learning history to a cloud account.

> [!NOTE]
> This README describes the **0.6.0 source**. Check the selected release tag when downloading. Screenshots were captured from the locally installed pre-release macOS build on **2026-09-03**, using its Simplified Chinese interface and existing local data. They are real app captures, not mockups.

![Current personal wordbook with source pronunciation, saved translations, recall scores, and detail actions](docs/screenshots/vocabulary-study-wordbook.jpg)

## What it does

- **Translate beside the selection.** A small overlay keeps the reading workflow in place; Quick Translate also works without selection access.
- **Choose one online provider.** Google Cloud Translation, Baidu Translate, or Microsoft Translator, including Microsoft's global and China cloud endpoints. There is no silent switch to another online provider.
- **Keep available meanings together.** Vocabulary cards display every saved translation with its known part of speech (POS). Pronunciation stays beside the source word.
- **Explore word details.** Open details from a vocabulary card to see its saved example, verified word parts, meanings, and matching textbook-word counts.
- **Learn locally.** Use an embedded English–Simplified Chinese starter, download additional textbooks, and practise word-to-meaning, meaning-to-word, or mixed questions.
- **Fit your desktop.** English and Simplified Chinese interfaces, system-voice pronunciation, a menu-bar / tray workflow, and optional start at login.

## Install and start

Download from [GitHub Releases](https://github.com/Ldsystem/desktop-translator/releases). Check the selected release's notes and assets; not every historical release includes both platforms.

| Platform | Packaging and support |
| --- | --- |
| macOS 11+ | The release workflow builds a universal Apple Silicon + Intel app and DMG. Drag the app to Applications and launch it. Current workflow artifacts use ad-hoc signing, not Developer ID notarization. |
| Windows 10/11 x64 | The release workflow builds an unsigned, current-user NSIS installer with WebView2 bootstrap support. See the [Windows distribution notes](docs/windows-signing-and-supply-chain.md). |
| Linux | Not implemented; the native crate rejects Linux builds. |

1. Open **Settings** from the app's menu-bar / tray menu.
2. On macOS, grant **Accessibility** access for selection translation. Quick Translate does not require reading another application's selection.
3. Choose source and target languages. For online translation, select a provider and configure its credentials.
4. Test the provider configuration. Select text and click Translate, or type into Quick Translate.
5. Open **Vocabulary Study** from the same menu to review your wordbook, practise, or browse textbooks.

| Provider | Configuration used by this app |
| --- | --- |
| Google Cloud Translation | Cloud Translation API key; this is not the Google Translate website. |
| Baidu Translate | APP ID and secret key. |
| Microsoft Translator | Subscription key, cloud selection, and region when required by the resource. |

Credentials are entered through native prompts and stored in the operating system's credential vault. Language availability and online errors depend on the selected provider. Local wordbook and active-textbook hits can be used without an online request.

> [!IMPORTANT]
> Selection support depends on what the other application exposes through accessibility APIs. Secure fields, some PDF viewers, terminals, and custom controls may not expose usable text. There is no OCR or automatic clipboard fallback; use Quick Translate when selection access is unavailable. Unsigned/ad-hoc release artifacts may trigger system security warnings—verify the download's origin before approving it.

## Vocabulary Study

### From a lookup to a word detail

Successful lookups of eligible words and short lexical phrases are saved locally. Sentence-like selections still translate, but are not added as vocabulary entries. When the accessibility selection provides a suitable surrounding sentence, it can be recorded as the word's example.

- **My wordbook:** search and review entries, see lookup demand and recall scores, correct or remove entries, and read all saved translations with available POS labels.
- **Word details:** use the detail action on a vocabulary card. The page contains the source word and pronunciation, the recorded example when available, verified composing roots/affixes, and individual meanings with connection counts.
- **Related words:** click a root or meaning to open that exact group. Back navigation returns to the word detail. Details and related words are contextual pages, not extra sidebar destinations.
- **Practice:** submit answers to update recall. Looking up a word increases lookup demand; it does not count as remembering it. Multiple meanings remain one vocabulary item rather than creating duplicate learning records.

| Word detail | Related meanings |
| --- | --- |
| ![Sublime detail: one saved adjective meaning, two related words, and no verified roots](docs/screenshots/vocabulary-study-detail.jpg) | ![Sublime related meanings: noble and supernal, with textbook origins](docs/screenshots/vocabulary-study-related.jpg) |

This example has one saved meaning. Entries with more available senses display those too; an empty root section means no verified structure is available.

### Where extra meanings and connections come from

| Source | Current behavior |
| --- | --- |
| Personal wordbook | Reuses the saved entry and can merge additional senses from compatible installed textbooks without a network call. |
| Active textbook | Provides matching local translations before the online provider; a hit is promoted into the personal wordbook. |
| Microsoft Translator | Adds dictionary alternatives and POS for eligible lookups in the app's supported dictionary language pairs, including English–Simplified Chinese. |
| Google Cloud / Baidu | Provide a primary online translation. Additional senses can come from installed textbooks; the app does not scrape either provider's consumer website. |
| Verified lexical data | Supplies curated word-part relationships. Unsupported words remain without verified roots; similar spelling alone is not presented as etymology. |

**Refresh meanings** is available in word details when Microsoft is selected and the language pair is supported. It is an explicit online action, not a background dictionary crawl. Existing saved meanings remain usable when enrichment is unavailable.

Counts beside a root or meaning represent **distinct matching words in compatible installed textbooks**, including inactive books, excluding the word itself. A word appearing in several books counts once and retains its origins. The related page uses the same filter as the count; a personal-wordbook match can annotate a result but does not add another textbook word to the count.

> [!NOTE]
> “All translations” means all available saved senses, not an exhaustive dictionary guarantee. Coverage varies by word, provider, language pair, and textbook. Older entries may have only one meaning, and unknown POS or roots are not invented. Connections use local structured data, not embeddings or an LLM.

### Textbook shelf

**Discover** includes an embedded English–Simplified Chinese starter and downloadable general-reference, everyday, academic, TOEIC, and business collections. Each collection shows its attribution and source link.

In **Downloaded**, each card separates the title and word count from **Browse words**, **Make active / Deactivate**, and **Remove**. When enrichment is available, **Refresh word details** appears in a separate maintenance row. Unavailable legacy downloads cannot be refreshed through an invalid catalog target.

![Downloaded textbooks with aligned learning actions, a separate refresh row, and a highlighted active dictionary](docs/screenshots/vocabulary-study-textbooks.jpg)

One textbook can be active for lookup and practice at a time. Other installed books still contribute to compatible connections. Downloaded word counts can differ from source word-list sizes because only validated, usable dictionary matches are imported. Downloads and refreshes validate their artifacts; a failed refresh does not discard the existing book.

## Privacy and data

- **Local storage:** non-secret preferences live in the app's configuration directory as `settings.json`. Vocabulary, senses, saved examples, textbook entries, and practice state live in the app-data SQLite database, `vocabulary.sqlite3`. There is no built-in account sync.
- **Selection access:** native accessibility APIs read selected text and may inspect surrounding text to extract a local example. Selection translation does not capture screenshots, run OCR, or automatically copy text to the clipboard.
- **Network boundary:** an online translation sends the requested text and language direction to the chosen provider. Explicit meaning refresh, provider testing/credential validation, language-list retrieval, and textbook download/refresh can also use the network. The saved example is not included in translation providers' request payloads.
- **Secrets:** credentials stay in native system-vault storage (macOS Keychain / Windows credential storage). The web interface receives credential status, not secret values.
- **Local learning:** wordbook lookup, installed-textbook browsing, related-word matching, and practice use local data. Native services own database access and return the records needed by the interface.

## Development

The application uses **Tauri 2 + Rust** for native services and **React 19 + TypeScript + Vite** for the interface. SQLite is bundled through `rusqlite`; no separate database server is needed.

Prerequisites:

- Node.js **22.12+ on the 22.x line**, or a compatible newer even-numbered release; pnpm **10**. Current Vite also accepts Node 20.19+, but not earlier Node 20 versions.
- Stable Rust and the target platform's native toolchain: Xcode Command Line Tools on macOS, or MSVC C++ build tools and Windows SDK on Windows.
- A WebView2 runtime to run the Windows app; the packaged installer can bootstrap it.

```sh
git clone https://github.com/Ldsystem/desktop-translator.git
cd desktop-translator
pnpm install --frozen-lockfile
pnpm tauri dev
```

The development app starts in the menu bar / tray. `pnpm dev` alone starts the frontend server; native selection, storage, speech, and credential features need the Tauri host. macOS development uses the repository's signing runner; see [development signing](docs/macos-development-signing.md) for a stable local identity and permission troubleshooting.

### Checks

```sh
pnpm check
pnpm test:platform
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
```

`pnpm check` runs TypeScript checks, frontend tests, and the frontend build. `pnpm test:platform` runs Rust tests on the current supported host. Native CI covers macOS and Windows, but interactive selection, multi-monitor, speech, and installer checks still need real-host validation. The [platform test matrix](docs/platform-test-matrix.md) contains recorded results and manual fixtures; historical entries are not proof that every current build has passed them.

### Build a local package

```sh
pnpm tauri build
```

Host-architecture bundles are written under `src-tauri/target/release/bundle/`. Building does not replace an installed app automatically. For a universal macOS package:

```sh
rustup target add aarch64-apple-darwin x86_64-apple-darwin
pnpm tauri build --target universal-apple-darwin
```

Universal bundles go under `src-tauri/target/universal-apple-darwin/release/bundle/`. The [release workflow](.github/workflows/release.yml) packages and audits macOS and Windows artifacts; signing and publication are separate from a successful local build.

### Code map

| Path | Responsibility |
| --- | --- |
| [src/components/](src/components/) | Selection overlay, Quick Translate, settings, vocabulary cards, details, practice, and textbook views. |
| [src/contracts/](src/contracts/) / [src-tauri/src/contracts.rs](src-tauri/src/contracts.rs) | Frontend/native request, response, validation, and error contracts. |
| [src-tauri/src/services/](src-tauri/src/services/) | Translation providers, credentials, settings, vocabulary persistence, textbooks, and study logic. |
| [src-tauri/src/platform/](src-tauri/src/platform/) | OS-specific selection, accessibility, window, and speech integration. |
| [src-tauri/resources/](src-tauri/resources/) | Bundled textbook, verified lexical metadata, and provider capability data. |
| [docs/](docs/) | Screenshots, platform qualification, and signing/distribution notes. |

Built with Tauri, React, Rust, and SQLite. Textbook sources include WikDict/Wiktionary/DBnary and the NGSL Project; collection-specific attribution remains visible in the app.
