#!/usr/bin/env bash
# Check .changes/unreleased/*.yaml, then run `changie batch auto --dry-run`.
#
# Tag release runs `changie batch` (see .github/workflows/tag-release.yml).
# An unquoted body containing ": " is invalid YAML, so batch stops. A missing
# `time` still batches, and a bad `kind` is reported without the filename.
# This script names the file, requires kind, body, and time, checks kind
# against .changie.yaml, then runs the same batch with --dry-run.
#
# Changie is the miniscruff/changie-action@v3 build with version: latest,
# which is what Tag release installs. --dry-run prints the batch and leaves
# the fragments in place.
#
#   scripts/check-changie-fragments.sh
#   scripts/check-changie-fragments.sh --self-test
#
# CHANGIE_CHECK_ROOT   tree to check (default: repo root)
# CHANGIE_BIN          changie binary (default: changie on PATH)
# CHANGIE_SKIP_BATCH=1 parse fragments only
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT="$SCRIPT_DIR/$(basename "${BASH_SOURCE[0]}")"

usage() {
  echo "Usage: scripts/check-changie-fragments.sh [--self-test]" >&2
  exit 1
}

if [ -n "${CHANGIE_CHECK_ROOT:-}" ]; then
  ROOT="$(cd "$CHANGIE_CHECK_ROOT" && pwd)"
else
  ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
fi
CHANGIE_BIN="${CHANGIE_BIN:-changie}"

# Prints the fragment count on stdout. Diagnostics go to stderr.
validate_fragments() {
  ruby - "$1" <<'RUBY'
require "date"
require "time"
require "yaml"

root = ARGV.fetch(0)

def rel(root, path)
  prefix = "#{root}/"
  path.start_with?(prefix) ? path.delete_prefix(prefix) : path
end

def load_document(raw)
  docs = Psych.parse_stream(raw).children
  raise "expected a single YAML document, found #{docs.length}" if docs.length != 1

  YAML.safe_load(raw, permitted_classes: [Time, Date, DateTime])
end

def timestamp?(value)
  case value
  when Time, DateTime, Date
    true
  when String
    return false if value.strip.empty?

    begin
      Time.iso8601(value)
      true
    rescue ArgumentError
      begin
        Date.iso8601(value)
        true
      rescue ArgumentError
        false
      end
    end
  else
    false
  end
end

def yaml_error(error)
  error.message.sub(/\A\(<unknown>\):\s*/, "")
end

config_path = File.join(root, ".changie.yaml")
config_rel = rel(root, config_path)
unless File.file?(config_path)
  $stderr.puts "#{config_rel}: file not found"
  exit 1
end

begin
  config = load_document(File.read(config_path))
rescue StandardError => e
  $stderr.puts "#{config_rel}: invalid YAML (#{yaml_error(e)})"
  exit 1
end

unless config.is_a?(Hash)
  $stderr.puts "#{config_rel}: must be a YAML mapping"
  exit 1
end

kinds = config["kinds"]
unless kinds.is_a?(Array) && !kinds.empty?
  $stderr.puts "#{config_rel}: kinds must be a non-empty list"
  exit 1
end

allowed = kinds.map do |kind|
  unless kind.is_a?(Hash)
    $stderr.puts "#{config_rel}: each kind must be a mapping with a label"
    exit 1
  end

  key = kind["key"]
  label = kind["label"]
  # Changie matches KeyOrLabel: the key when it is set, otherwise the label.
  if key.is_a?(String) && !key.empty?
    key
  elsif label.is_a?(String) && !label.empty?
    label
  else
    $stderr.puts "#{config_rel}: a kind is missing label"
    exit 1
  end
end

dir = File.join(root, ".changes", "unreleased")
names = []
names = Dir.children(dir).select { |name| File.extname(name) == ".yaml" }.sort if Dir.exist?(dir)

if names.empty?
  $stderr.puts "No unreleased Changie fragments."
  puts "0"
  exit 0
end

errors = []
names.each do |name|
  path = File.join(dir, name)
  shown = rel(root, path)
  begin
    data = load_document(File.read(path))
  rescue Psych::SyntaxError => e
    errors << "#{shown}: invalid YAML (#{yaml_error(e)}). Quote values that contain ': '."
    next
  rescue StandardError => e
    errors << "#{shown}: invalid YAML (#{yaml_error(e)})"
    next
  end

  unless data.is_a?(Hash)
    errors << "#{shown}: must be a YAML mapping with kind, body, and time"
    next
  end

  %w[kind body time].each do |field|
    errors << "#{shown}: missing #{field}" if !data.key?(field) || data[field].nil?
  end

  kind = data["kind"]
  if data.key?("kind") && !kind.nil?
    if kind.is_a?(String) && !kind.empty?
      unless allowed.include?(kind)
        errors << "#{shown}: kind #{kind.inspect} is not in .changie.yaml (#{allowed.join(", ")})"
      end
    else
      errors << "#{shown}: kind must be a non-empty string"
    end
  end

  body = data["body"]
  if data.key?("body") && !body.nil? && !(body.is_a?(String) && !body.strip.empty?)
    errors << "#{shown}: body must be a non-empty string. Quote it when it contains ': '."
  end

  time = data["time"]
  if data.key?("time") && !time.nil? && !timestamp?(time)
    errors << "#{shown}: time must be a timestamp such as 2026-11-15T02:45:00Z"
  end
end

unless errors.empty?
  errors.each { |message| $stderr.puts message }
  exit 1
end

noun = names.length == 1 ? "fragment" : "fragments"
$stderr.puts "Validated #{names.length} Changie #{noun}."
puts names.length.to_s
RUBY
}

changie_installed() {
  if [[ "$CHANGIE_BIN" == */* ]]; then
    [ -x "$CHANGIE_BIN" ]
  else
    command -v "$CHANGIE_BIN" >/dev/null 2>&1
  fi
}

run_check() {
  local count
  if ! count="$(validate_fragments "$ROOT")"; then
    exit 1
  fi
  case "$count" in
    "" | *[!0-9]*)
      echo "fragment check returned an unexpected count: ${count}" >&2
      exit 1
      ;;
  esac

  if [ "$count" -eq 0 ] || [ "${CHANGIE_SKIP_BATCH:-}" = "1" ]; then
    exit 0
  fi

  if ! changie_installed; then
    echo "changie is required for 'changie batch auto --dry-run'. Install miniscruff/changie-action@v3 with version latest, the same as Tag release." >&2
    exit 1
  fi

  cd "$ROOT"
  echo "Running ${CHANGIE_BIN} batch auto --dry-run"
  if ! "$CHANGIE_BIN" batch auto --dry-run; then
    echo "changie batch auto --dry-run failed." >&2
    exit 1
  fi
}

self_test() {
  local tmp repo log fake out
  tmp="$(mktemp -d)"
  # Expand the path now. An EXIT trap that reads the local runs after the
  # function returns, when `tmp` is unset and `set -u` fails the script.
  # shellcheck disable=SC2064
  trap "rm -rf $(printf '%q' "$tmp")" EXIT
  repo="$tmp/repo"
  log="$tmp/changie-args"
  fake="$tmp/bin/changie"
  mkdir -p "$repo/.changes/unreleased" "$(dirname "$fake")"
  cp "$ROOT/.changie.yaml" "$repo/.changie.yaml"
  # A caller (CI) may point CHANGIE_BIN at a real binary. These cases choose.
  unset CHANGIE_BIN

  reset_fragments() {
    rm -f "$repo/.changes/unreleased/"*.yaml
  }

  write_fragment() {
    local name="$1"
    cat > "$repo/.changes/unreleased/$name"
  }

  expect_ok() {
    local label="$1"
    shift
    out="$tmp/out"
    if ! "$@" >"$out" 2>&1; then
      echo "self-test: ${label} failed" >&2
      cat "$out" >&2
      exit 1
    fi
  }

  expect_fail() {
    local label="$1"
    local needle="$2"
    shift 2
    out="$tmp/out"
    if "$@" >"$out" 2>&1; then
      echo "self-test: ${label} was accepted" >&2
      cat "$out" >&2
      exit 1
    fi
    if ! grep -F -- "$needle" "$out" >/dev/null; then
      echo "self-test: ${label} did not report: ${needle}" >&2
      cat "$out" >&2
      exit 1
    fi
  }

  write_fragment "Fixed-good.yaml" <<'EOF'
kind: Fixed
body: "acme: quote a colon followed by a space"
time: 2026-11-15T00:00:00Z
EOF
  expect_ok "quoted colon" env CHANGIE_CHECK_ROOT="$repo" CHANGIE_SKIP_BATCH=1 "$SCRIPT"
  if ! grep -F "Validated 1 Changie fragment." "$out" >/dev/null; then
    echo "self-test: success message missing" >&2
    cat "$out" >&2
    exit 1
  fi

  write_fragment "Fixed-block.yaml" <<'EOF'
kind: Fixed
body: |
  acme: a block scalar is a string
time: 2026-11-15T00:00:00Z
EOF
  expect_ok "block scalar" env CHANGIE_CHECK_ROOT="$repo" CHANGIE_SKIP_BATCH=1 "$SCRIPT"

  write_fragment "Fixed-bad.yaml" <<'EOF'
kind: Fixed
body: acme: unquoted colon
time: 2026-11-15T00:00:00Z
EOF
  expect_fail "unquoted colon" ".changes/unreleased/Fixed-bad.yaml:" \
    env CHANGIE_CHECK_ROOT="$repo" CHANGIE_SKIP_BATCH=1 "$SCRIPT"
  if ! grep -F "Quote values that contain ': '." "$out" >/dev/null; then
    echo "self-test: unquoted colon did not explain the quote" >&2
    cat "$out" >&2
    exit 1
  fi
  if grep -F "Fixed-good.yaml:" "$out" >/dev/null || grep -F "Fixed-block.yaml:" "$out" >/dev/null; then
    echo "self-test: a valid fragment was reported as bad" >&2
    cat "$out" >&2
    exit 1
  fi

  reset_fragments
  write_fragment "Fixed-missing.yaml" <<'EOF'
kind: Fixed
EOF
  expect_fail "missing body" ".changes/unreleased/Fixed-missing.yaml: missing body" \
    env CHANGIE_CHECK_ROOT="$repo" CHANGIE_SKIP_BATCH=1 "$SCRIPT"
  expect_fail "missing time" ".changes/unreleased/Fixed-missing.yaml: missing time" \
    env CHANGIE_CHECK_ROOT="$repo" CHANGIE_SKIP_BATCH=1 "$SCRIPT"

  reset_fragments
  write_fragment "Fixed-kind.yaml" <<'EOF'
kind: Bugfix
body: not a configured kind
time: 2026-11-15T00:00:00Z
EOF
  expect_fail "unknown kind" ".changes/unreleased/Fixed-kind.yaml: kind \"Bugfix\" is not in .changie.yaml" \
    env CHANGIE_CHECK_ROOT="$repo" CHANGIE_SKIP_BATCH=1 "$SCRIPT"

  reset_fragments
  write_fragment "Fixed-time.yaml" <<'EOF'
kind: Fixed
body: bad timestamp
time: not-a-time
EOF
  expect_fail "bad time" ".changes/unreleased/Fixed-time.yaml: time must be a timestamp" \
    env CHANGIE_CHECK_ROOT="$repo" CHANGIE_SKIP_BATCH=1 "$SCRIPT"

  reset_fragments
  cat > "$fake" <<EOF
#!/bin/sh
printf '%s\n' "\$*" >> "$log"
exit 0
EOF
  chmod +x "$fake"
  : >"$log"
  expect_ok "no fragments" env CHANGIE_CHECK_ROOT="$repo" CHANGIE_BIN="$fake" "$SCRIPT"
  if [ -s "$log" ]; then
    echo "self-test: changie batch ran with no fragments" >&2
    cat "$log" >&2
    exit 1
  fi

  write_fragment "Fixed-good.yaml" <<'EOF'
kind: Fixed
body: "acme: quote a colon followed by a space"
time: 2026-11-15T00:00:00Z
EOF
  expect_ok "batch dry-run" env CHANGIE_CHECK_ROOT="$repo" CHANGIE_BIN="$fake" "$SCRIPT"
  if ! grep -Fx "batch auto --dry-run" "$log" >/dev/null; then
    echo "self-test: expected 'changie batch auto --dry-run'" >&2
    cat "$log" >&2
    exit 1
  fi

  cat > "$fake" <<'EOF'
#!/bin/sh
echo "kind not found but configuration expects one: 'Bugfix'" >&2
exit 1
EOF
  chmod +x "$fake"
  expect_fail "changie failure" "changie batch auto --dry-run failed." \
    env CHANGIE_CHECK_ROOT="$repo" CHANGIE_BIN="$fake" "$SCRIPT"
  if ! grep -F "kind not found but configuration expects one: 'Bugfix'" "$out" >/dev/null; then
    echo "self-test: changie stderr was dropped" >&2
    cat "$out" >&2
    exit 1
  fi

  echo "self-test passed"
}

case "${1:-}" in
  "")
    run_check
    ;;
  --self-test)
    self_test
    ;;
  *)
    usage
    ;;
esac
