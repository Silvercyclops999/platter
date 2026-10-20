#!/bin/sh
# Fails if any commit in the range has a trailer (Co-Authored-By, Signed-off-by,
# anything in "Key: value" form at the end of the message) or wasn't written by
# the project's noreply address. Usage: scripts/check-commits.sh <base>..<head>
range=${1:?usage: check-commits.sh base..head}
author=291674526+Silvercyclops999@users.noreply.github.com
commits=$(git rev-list --no-merges "$range") || exit 1
bad=0
for c in $commits; do
  ae=$(git log -1 --format=%ae "$c")
  ce=$(git log -1 --format=%ce "$c")
  subject=$(git log -1 --format=%s "$c")
  short=$(git log -1 --format=%h "$c")
  if [ "$ae" != "$author" ]; then
    echo "$short has another author: $ae ($subject)"; bad=1
  fi
  # GitHub itself commits as noreply@github.com when a branch is updated on the site.
  if [ "$ce" != "$author" ] && [ "$ce" != "noreply@github.com" ]; then
    echo "$short has another committer: $ce ($subject)"; bad=1
  fi
  # Real trailers, plus the usual ones anywhere in the message.
  trailers=$( { git log -1 --format='%(trailers:only,unfold)' "$c"
    git log -1 --format=%B "$c" | grep -iE '^(co-authored-by|claude-session|signed-off-by):'; } | sed '/^$/d' | sort -u)
  if [ -n "$trailers" ]; then
    echo "$short has trailers ($subject):"; echo "$trailers" | sed 's/^/    /'; bad=1
  fi
done
if [ $bad -ne 0 ]; then
  echo
  echo "Fix your own branch with this, then force-push it:"
  echo "  git rebase <base> -x 'git log -1 --format=%B | grep -viE \"^(co-authored-by|claude-session|signed-off-by):\" | git commit --amend -q --reset-author -F -'"
  exit 1
fi
echo "All commits in $range are clean."
