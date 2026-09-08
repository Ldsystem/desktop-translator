#!/usr/bin/env bash

set -euo pipefail

readonly github_owner="Ldsystem"
readonly github_repo="desktop-translator"
readonly gitee_owner="shenglongliu"
readonly gitee_repo="desktop-translator"
readonly gitee_api="https://gitee.com/api/v5"

for command_name in curl git jq mktemp sort; do
  command -v "$command_name" >/dev/null 2>&1 || {
    echo "Required command is not available: $command_name" >&2
    exit 1
  }
done

: "${GITEE_PRIVATE_SECRET:?Set GITEE_PRIVATE_SECRET as a sensitive Gitee Go environment variable}"

work_dir="$(mktemp -d "${TMPDIR:-/tmp}/desktop-translator-gitee.XXXXXX")"
trap 'rm -rf "$work_dir"' EXIT

asset_dir="$work_dir/assets"
mkdir -p "$asset_dir"

urlencode() {
  jq -rn --arg value "$1" '$value | @uri'
}

find_release_tag() {
  local commit target_tag

  if [[ -n "${RELEASE_TAG:-}" ]]; then
    printf '%s\n' "$RELEASE_TAG"
    return
  fi

  commit="${GITEE_COMMIT:-HEAD}"
  target_tag="$(git tag --points-at "$commit" --list 'v*' | sort -V | tail -n 1 || true)"
  if [[ -z "$target_tag" ]]; then
    # Gitee Go may use a shallow checkout. A tag fetch is safe here because
    # this repository is public and makes manual runs on main deterministic.
    git fetch --quiet --tags origin || true
    target_tag="$(git tag --points-at "$commit" --list 'v*' | sort -V | tail -n 1 || true)"
  fi
  if [[ -z "$target_tag" ]]; then
    target_tag="$(git describe --tags --abbrev=0 --match 'v*' 2>/dev/null || true)"
  fi

  printf '%s\n' "$target_tag"
}

release_tag="$(find_release_tag)"
if [[ ! "$release_tag" =~ ^v[0-9]+(\.[0-9]+)*([.-][0-9A-Za-z.-]+)?$ ]]; then
  echo "Could not determine a valid release tag (got: ${release_tag:-empty})" >&2
  echo "Run the pipeline on a v* tag or set RELEASE_TAG explicitly for a manual run." >&2
  exit 1
fi

github_tag="$(urlencode "$release_tag")"
github_release_file="$work_dir/github-release.json"
github_release_url="https://api.github.com/repos/${github_owner}/${github_repo}/releases/tags/${github_tag}"

echo "Fetching GitHub release metadata for $release_tag"
curl --fail-with-body --silent --show-error \
  --connect-timeout 30 \
  --max-time 120 \
  --retry 4 \
  --retry-all-errors \
  --retry-delay 2 \
  --header "Accept: application/vnd.github+json" \
  --header "User-Agent: desktop-translator-gitee-release" \
  --output "$github_release_file" \
  "$github_release_url"

jq -e '.draft == false and (.assets | type == "array") and (.assets | length > 0)' \
  "$github_release_file" >/dev/null || {
  echo "GitHub release $release_tag is missing or has no published assets" >&2
  exit 1
}

github_release_name="$(jq -er --arg tag "$release_tag" \
  '.name // ("Desktop Translator " + $tag)' "$github_release_file")"
github_release_body="$(jq -r '.body // empty' "$github_release_file")"
release_body="${github_release_body}"
if [[ -n "$release_body" ]]; then
  release_body+=$'\n\n'
fi
release_body+="Source GitHub release: https://github.com/${github_owner}/${github_repo}/releases/tag/${release_tag}"

echo "Downloading GitHub release assets"
while IFS=$'\t' read -r asset_name asset_url; do
  if [[ -z "$asset_name" || "$asset_name" == */* || "$asset_name" == "." || "$asset_name" == ".." ]]; then
    echo "GitHub returned an unsafe asset name: $asset_name" >&2
    exit 1
  fi

  asset_file="$asset_dir/$asset_name"
  rm -f "$asset_file"
  curl --fail-with-body --silent --show-error \
    --location \
    --connect-timeout 30 \
    --max-time 900 \
    --retry 4 \
    --retry-all-errors \
    --retry-delay 2 \
    --header "Accept: application/octet-stream" \
    --header "User-Agent: desktop-translator-gitee-release" \
    --output "$asset_file" \
    "$asset_url"
  echo "Downloaded $asset_name"
done < <(jq -r '.assets[] | [.name, .browser_download_url] | @tsv' "$github_release_file")

gitee_release_file="$work_dir/gitee-release.json"
gitee_release_url="$gitee_api/repos/${gitee_owner}/${gitee_repo}/releases/tags/${github_tag}"

echo "Looking up Gitee release $release_tag"
if ! lookup_status="$(curl --silent --show-error \
  --connect-timeout 30 \
  --max-time 120 \
  --header "Accept: application/json" \
  --header "Authorization: Bearer ${GITEE_PRIVATE_SECRET}" \
  --output "$gitee_release_file" \
  --write-out '%{http_code}' \
  "$gitee_release_url")"; then
  echo "Gitee release lookup failed" >&2
  exit 1
fi

if [[ "$lookup_status" == "200" ]] &&
  jq -e 'type == "object" and (.id != null)' "$gitee_release_file" >/dev/null; then
  echo "Gitee release $release_tag already exists; reusing it"
elif [[ "$lookup_status" == "404" ]] || {
  [[ "$lookup_status" == "200" ]] &&
    jq -e 'type == "null" or (type == "object" and .id == null)' "$gitee_release_file" >/dev/null
}; then
  curl --fail-with-body --silent --show-error \
    --connect-timeout 30 \
    --max-time 120 \
    --request POST \
    --header "Accept: application/json" \
    --header "Authorization: Bearer ${GITEE_PRIVATE_SECRET}" \
    --data-urlencode "tag_name=${release_tag}" \
    --data-urlencode "name=${github_release_name}" \
    --data-urlencode "body=${release_body}" \
    --data-urlencode "target_commitish=main" \
    --output "$gitee_release_file" \
    "$gitee_api/repos/${gitee_owner}/${gitee_repo}/releases"
  echo "Created Gitee release $release_tag"
else
  echo "Gitee release lookup failed (HTTP ${lookup_status})" >&2
  exit 1
fi

release_id="$(jq -er '.id' "$gitee_release_file")"
attachments_url="$gitee_api/repos/${gitee_owner}/${gitee_repo}/releases/${release_id}/attach_files"
attachments_file="$work_dir/gitee-attachments.json"

refresh_attachments() {
  curl --fail-with-body --silent --show-error \
    --connect-timeout 30 \
    --max-time 120 \
    --header "Accept: application/json" \
    --header "Authorization: Bearer ${GITEE_PRIVATE_SECRET}" \
    --output "$attachments_file" \
    "${attachments_url}?per_page=100"
  jq -e 'type == "array"' "$attachments_file" >/dev/null
}

refresh_attachments

for asset_file in "$asset_dir"/*; do
  [[ -f "$asset_file" ]] || continue
  asset_name="${asset_file##*/}"

  if jq -e --arg name "$asset_name" 'any(.[]; .name == $name)' "$attachments_file" >/dev/null; then
    echo "Gitee already has $asset_name; skipping"
    continue
  fi

  echo "Uploading $asset_name to Gitee"
  upload_file="$work_dir/upload-${asset_name}.json"
  upload_status=0
  if curl --fail-with-body --silent --show-error \
    --connect-timeout 30 \
    --max-time 900 \
    --http1.1 \
    --request POST \
    --header "Accept: application/json" \
    --header "Authorization: Bearer ${GITEE_PRIVATE_SECRET}" \
    --header "Expect:" \
    --form "access_token=${GITEE_PRIVATE_SECRET}" \
    --form "file=@${asset_file}" \
    --output "$upload_file" \
    "$attachments_url"; then
    :
  else
    upload_status=$?
    echo "Gitee upload request failed for $asset_name (curl exit ${upload_status}); checking attachment state" >&2
  fi

  # A multipart request can complete server-side after the client loses its
  # response. Re-read the release before reporting failure, so a rerun is only
  # needed when Gitee genuinely does not have the asset.
  refresh_attachments
  if jq -e --arg name "$asset_name" 'any(.[]; .name == $name)' "$attachments_file" >/dev/null; then
    echo "Gitee confirmed $asset_name"
  else
    if [[ "$upload_status" -ne 0 ]]; then
      echo "Gitee did not receive $asset_name after the failed upload request" >&2
    else
      echo "Gitee upload returned successfully but $asset_name is not listed" >&2
    fi
    exit 1
  fi
done

echo "Gitee release $release_tag is ready with all GitHub release assets"
