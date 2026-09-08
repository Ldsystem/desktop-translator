# Release and Gitee mirror

The release flow has two hosts with separate responsibilities:

1. GitHub Actions builds the macOS universal and Windows x64 binaries and
   creates the GitHub Release when a `v*` tag is pushed.
2. Gitee mirrors the source repository and tags. Gitee Go then downloads the
   public GitHub Release assets and publishes them as attachments on the
   matching Gitee release.

This keeps the upload request inside Gitee instead of sending a large
multipart request from a GitHub runner to Gitee.

## One-time Gitee setup

After the workflow files have reached the mirrored Gitee repository:

1. Enable Gitee Go for `shenglongliu/desktop-translator` and create the
   pipeline from `.workflow/gitee-release.yml`.
2. Add a Gitee Go environment variable named `GITEE_PRIVATE_SECRET`.
3. Put the existing Gitee repository token in that variable and mark it as
   sensitive/masked. The GitHub repository secret with the same name is not
   automatically visible to Gitee Go.
4. Run the pipeline once manually to bootstrap the current `v0.6.0` release.
   The script uses the exact tag for a tag-triggered run, or the latest tag
   for a manual run on the default branch.

Future `v*` tags will trigger the Gitee pipeline after the mirror receives
the tag. The script reuses an existing Gitee release and skips attachments
that are already present, so rerunning a partially completed release is
safe.

The Gitee Go pipeline deliberately leaves its pipeline timeout unset. Each
individual download/upload request has a bounded network timeout, and an
upload timeout is followed by an attachment-state check rather than a blind
retry.
