# Release and Gitee mirror

The release flow has two hosts with separate responsibilities:

1. GitHub Actions builds the macOS universal and Windows x64 binaries and
   creates the GitHub Release when a `v*` tag is pushed.
2. GitHub Actions copies those assets to a dedicated `gitee-release-v*` Git
   branch and creates or updates the matching Gitee release page.
3. Gitee's repository mirror copies that branch, so the release page's links
   resolve to files hosted by Gitee.

This uses ordinary Git transport instead of Gitee's large multipart release
attachment endpoint. It also avoids requiring Gitee Go.

## One-time Gitee setup

The existing repository mirror must synchronize branches, tags, and commits
from GitHub. The Gitee mirror feature supports those Git objects; Release
attachments are handled by the GitHub workflow separately.

The existing GitHub repository secret is sufficient:

1. Keep `GITEE_PRIVATE_SECRET` in GitHub Actions. It is used only for the
   small Gitee release create/update API request; it is not used to upload
   binary data.
2. Ensure the Gitee mirror is configured to synchronize all branches, not
   only `main`, so it receives branches named `gitee-release-v*`.
3. Run the GitHub Release workflow once manually for `v0.6.0` to bootstrap the
   current release. The workflow will create `gitee-release-v0.6.0`; after the
   mirror catches up, its files will be downloadable from the Gitee release
   page.

Future `v*` tags will build the GitHub release and create a corresponding
`gitee-release-v*` branch automatically. Re-running a release safely replaces
that exact generated branch and updates the release links.

The trade-off is that each release binary is stored once in the GitHub Git
repository in addition to the GitHub Release storage. The current artifacts
are well below Gitee's per-file limit, but old release branches should be
cleaned up if repository size becomes a concern.
