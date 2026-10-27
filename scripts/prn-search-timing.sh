#!/usr/bin/env bash
# prn-search-timing.sh: time PR-Nexus's sync queries against GitHub. Read-only.
#
# It sends the same GraphQL that PR-Nexus sync runs, through `gh api graphql`
# with your gh login:
#   1. the slim search page (graphql/search.graphql: $query, $first, $cursor),
#      with the search string narrowed to your orgs like the app does;
#   2. the status query (graphql/status.graphql) on the ids that page
#      returned: review decision, mergeability, checks, review threads.
# For each run it prints the HTTP status and seconds of both requests, their
# sum, the number of PRs returned, hasNextPage and the rate-limit cost.
# It prints no PR titles, URLs or other PR data. It only reads (queries, no
# mutations), first page only (no cursor).
#
# Needs: gh (logged in: `gh auth status`) and perl (built into macOS).
#
# Usage:
#   bash prn-search-timing.sh --org acme --org acme-labs
#                                  # the two buckets, scoped to those orgs (ORed)
#   bash prn-search-timing.sh --org acme --user octocat
#                                  # ...plus your personal repos (the app's toggle)
#   bash prn-search-timing.sh --org acme --old
#                                  # ...and the old full search for comparison
#   bash prn-search-timing.sh --sizes "10 5 3" --repeat 3
#   bash prn-search-timing.sh 'is:pr is:open repo:acme/app archived:false sort:updated-desc'
#                                  # only the search strings given as arguments
#
# Options:
#   --org NAME       add ` org:NAME` to each bucket (repeatable; ORed by GitHub)
#   --user LOGIN     add ` user:LOGIN` (personal repos)
#   --involves       also time `is:pr is:open involves:@me archived:false sort:updated-desc`
#   --old            also time the old search that carried every field
#   --sizes "A B C"  page sizes to try (default "10 5 3", the app's sizes)
#   --repeat N       runs per size (default 1)
#   --host HOST      GitHub host for gh (default github.com)
#
# Like the app, qualifiers spill into more search strings so none passes
# GitHub's 256-character limit. Without --org or --user the buckets run
# unscoped (the app searches nothing until you pick orgs).
#
# Reading the output: GitHub stops a GraphQL query after about 10 s and answers
# 502 with an HTML page. The app gives each request 15 s, then retries smaller
# (10 -> 5 -> 3). A size that shows 502 or ~10 s is too large for that query.

set -euo pipefail

MINE='is:pr is:open author:@me archived:false sort:updated-desc'
REVIEW='is:pr is:open review-requested:@me archived:false sort:updated-desc'
INVOLVES='is:pr is:open involves:@me archived:false sort:updated-desc'
MAX_QUERY=256

sizes="10 5 3"
repeat=1
host="github.com"
involves=0
old=0
owners=()
custom=()

while [ $# -gt 0 ]; do
  case "$1" in
    --involves) involves=1; shift ;;
    --old) old=1; shift ;;
    --org) owners+=("org:$2"); shift 2 ;;
    --user) owners+=("user:$2"); shift 2 ;;
    --sizes) sizes="$2"; shift 2 ;;
    --repeat) repeat="$2"; shift 2 ;;
    --host) host="$2"; shift 2 ;;
    -h|--help) sed -n '2,45p' "$0"; exit 0 ;;
    --*) echo "unknown option: $1" >&2; exit 2 ;;
    *) custom+=("$1"); shift ;;
  esac
done

command -v gh >/dev/null || { echo "gh is not installed" >&2; exit 1; }
command -v perl >/dev/null || { echo "perl is not installed" >&2; exit 1; }
gh auth status --hostname "$host" >/dev/null 2>&1 || { echo "gh is not logged in to $host (run: gh auth login)" >&2; exit 1; }

# Exact copy of search.graphql.
read -r -d '' SLIM <<'GRAPHQL' || true
query SearchPullRequests($query: String!, $first: Int!, $cursor: String) {
  rateLimit {
    limit
    remaining
    resetAt
    cost
  }
  search(type: ISSUE, query: $query, first: $first, after: $cursor) {
    pageInfo {
      hasNextPage
      endCursor
    }
    nodes {
      ... on PullRequest {
        ...PrListFields
      }
    }
  }
}

# Cheap fields only. Review decision, mergeability, checks and review threads
# are slow for GitHub to compute; sync reads them with status.graphql.
fragment PrListFields on PullRequest {
  id
  number
  title
  url
  bodyText
  isDraft
  state
  createdAt
  updatedAt
  author {
    login
  }
  headRefName
  baseRefName
  headRefOid
  repository {
    id
    owner {
      login
    }
    name
    isArchived
    defaultBranchRef {
      name
    }
  }
  labels(first: 20) {
    nodes {
      name
    }
  }
  reviewRequests(first: 20) {
    nodes {
      requestedReviewer {
        ... on User {
          login
        }
        ... on Team {
          slug
          organization {
            login
          }
        }
      }
    }
  }
}
GRAPHQL

# Exact copy of status.graphql.
read -r -d '' STATUS <<'GRAPHQL' || true
query PullRequestStatus($ids: [ID!]!) {
  rateLimit {
    limit
    remaining
    resetAt
    cost
  }
  nodes(ids: $ids) {
    ... on PullRequest {
      id
      reviewDecision
      mergeable
      mergeStateStatus
      commits(last: 1) {
        nodes {
          commit {
            committedDate
            statusCheckRollup {
              state
            }
          }
        }
      }
      reviewThreads(first: 100) {
        totalCount
        nodes {
          isResolved
        }
      }
    }
  }
}
GRAPHQL

# The search before slim search: every field in one query (for --old).
read -r -d '' FULL <<'GRAPHQL' || true
query SearchPullRequests($query: String!, $first: Int!, $cursor: String) {
  rateLimit {
    limit
    remaining
    resetAt
    cost
  }
  search(type: ISSUE, query: $query, first: $first, after: $cursor) {
    pageInfo {
      hasNextPage
      endCursor
    }
    nodes {
      ... on PullRequest {
        ...PrFields
      }
    }
  }
}

fragment PrFields on PullRequest {
  id
  number
  title
  url
  bodyText
  isDraft
  state
  createdAt
  updatedAt
  author {
    login
  }
  headRefName
  baseRefName
  headRefOid
  repository {
    id
    owner {
      login
    }
    name
    isArchived
    defaultBranchRef {
      name
    }
  }
  labels(first: 20) {
    nodes {
      name
    }
  }
  reviewDecision
  mergeable
  mergeStateStatus
  commits(last: 1) {
    nodes {
      commit {
        committedDate
        statusCheckRollup {
          state
        }
      }
    }
  }
  reviewRequests(first: 20) {
    nodes {
      requestedReviewer {
        ... on User {
          login
        }
        ... on Team {
          slug
          organization {
            login
          }
        }
      }
    }
  }
  reviewThreads(first: 100) {
    totalCount
    nodes {
      isResolved
    }
  }
}
GRAPHQL
export SLIM STATUS FULL

# Same packing as sync::search_queries: valid names only, no repeats
# (case-insensitive), as many qualifiers per string as fit in 256 characters.
pack() {
  local base="$1"; shift
  BASE="$base" MAX="$MAX_QUERY" perl -e '
    my ($base, $max) = ($ENV{BASE}, $ENV{MAX});
    my (@terms, %seen);
    for my $o (@ARGV) {
      my ($kind, $name) = split /:/, $o, 2;
      next unless defined $name && $name =~ /\A[A-Za-z0-9._-]+\z/;
      my $t = " $kind:$name";
      next if $seen{lc $t}++;
      push @terms, $t;
    }
    my $cur = $base;
    for my $t (@terms) {
      if (length($cur) + length($t) > $max && length($cur) > length($base)) {
        print "$cur\n"; $cur = $base;
      }
      $cur .= $t;
    }
    print "$cur\n";
  ' "$@"
}

searches=()
if [ ${#custom[@]} -gt 0 ]; then
  searches=("${custom[@]}")
else
  base=("$MINE" "$REVIEW")
  [ "$involves" = 1 ] && base+=("$INVOLVES")
  for b in "${base[@]}"; do
    while IFS= read -r line; do searches+=("$line"); done < <(pack "$b" ${owners[@]+"${owners[@]}"})
  done
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

now() { perl -MTime::HiRes=time -e 'printf "%.3f", time'; }

# post DOCUMENT_VAR VARIABLES_JSON -> $tmp/out, $tmp/err, prints elapsed seconds
post() {
  DOC_NAME="$1" VARS="$2" perl -MJSON::PP -e '
    print JSON::PP->new->canonical->encode({
      query => $ENV{ $ENV{DOC_NAME} },
      variables => decode_json($ENV{VARS}),
    });' > "$tmp/body.json"
  local start end
  start="$(now)"
  gh api graphql --hostname "$host" -i --input "$tmp/body.json" > "$tmp/out" 2> "$tmp/err" || true
  end="$(now)"
  perl -e 'printf "%.2f", $ARGV[1] - $ARGV[0]' "$start" "$end"
}

# read the last answer: prints "status|prs|more|cost/remaining|note|ids-json"
answer() {
  perl -MJSON::PP -e '
    my ($out, $err) = @ARGV;
    local $/; open my $fh, "<", $out; my $raw = <$fh> // ""; close $fh;
    open my $eh, "<", $err; my $stderr = <$eh> // ""; close $eh;
    my ($status) = $raw =~ m{^HTTP/\S+\s+(\d+)}m;
    ($status) = $stderr =~ m{HTTP\s+(\d+)} unless $status;
    $status //= "?";
    my ($body) = $raw =~ m{\r?\n\r?\n(.*)\z}s; $body //= "";
    my ($prs, $more, $rate, $note, @ids) = ("-", "-", "-", "");
    my $doc = eval { decode_json($body) };
    if ($doc) {
      my $d = $doc->{data} // {};
      my $nodes = $d->{search} ? $d->{search}{nodes} : $d->{nodes};
      if ($nodes) {
        my @live = grep { ref $_ && $_->{id} } @$nodes;
        $prs = scalar @live;
        @ids = map { $_->{id} } @live;
      }
      $more = $d->{search}{pageInfo}{hasNextPage} ? "yes" : "no" if $d->{search};
      $rate = "$d->{rateLimit}{cost}/$d->{rateLimit}{remaining}" if $d->{rateLimit};
      $note = "graphql error: " . substr($doc->{errors}[0]{message} // "", 0, 90) if $doc->{errors};
    } elsif ($body =~ /<html/i) {
      $note = "HTML error page (GitHub gave up on the query)";
    } elsif ($stderr ne "") {
      (my $line = $stderr) =~ s/\s+/ /g; $note = substr($line, 0, 100);
    }
    print join("\x1f", $status, $prs, $more, $rate, $note, encode_json(\@ids)), "\n";
  ' "$tmp/out" "$tmp/err"
}

row() { printf '%-5s %-4s %-6s %7s %-6s %7s %7s %4s %-4s %s\n' "$@"; }

for q in "${searches[@]}"; do
  echo
  echo "search (${#q} chars): $q"
  row size run search secs status secs total prs more "note"
  for size in $sizes; do
    for run in $(seq 1 "$repeat"); do
      vars="$(SEARCH="$q" FIRST="$size" perl -MJSON::PP -e 'print encode_json({query => $ENV{SEARCH}, first => $ENV{FIRST} + 0, cursor => undef})')"
      s_secs="$(post SLIM "$vars")"
      IFS=$'\x1f' read -r s_status prs more s_rate s_note ids < <(answer)
      st_status="-"; st_secs="0.00"; st_note=""
      if [ "$ids" != "[]" ] && [ -n "$ids" ]; then
        st_secs="$(post STATUS "{\"ids\":$ids}")"
        IFS=$'\x1f' read -r st_status _ _ _ st_note _ < <(answer)
      fi
      total="$(perl -e 'printf "%.2f", $ARGV[0] + $ARGV[1]' "$s_secs" "$st_secs")"
      note="$s_note${st_note:+ status: $st_note}"
      row "$size" "$run" "$s_status" "$s_secs" "$st_status" "$st_secs" "$total" "$prs" "$more" "$note"
      if [ "$old" = 1 ]; then
        o_secs="$(post FULL "$vars")"
        IFS=$'\x1f' read -r o_status o_prs o_more _ o_note _ < <(answer)
        row "" "old" "$o_status" "$o_secs" "" "" "$o_secs" "$o_prs" "$o_more" "$o_note"
      fi
    done
  done
done
