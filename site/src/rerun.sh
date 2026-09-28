#!/bin/sh
# Check the code as it is twice and compare the two runs: a rerun of
# unchanged code is answered from JevGate's cache, so it sends no request,
# costs nothing and reports what the first check did, down to each answer's
# probabilities. Run it where you run `jevgate check`; its arguments go to
# both checks (for example `sh rerun.sh --rule all`). Needs jq.
#
# Exit status: 0 the rerun sent nothing and matched, 1 it differed or sent
# requests, 2 a check did not finish or a tool is missing.
set -u

for tool in jevgate jq; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "rerun.sh needs $tool on PATH" >&2
        exit 2
    fi
done

# JevGate keeps its report in .jevgate/ at the repository root: the nearest
# directory up from here that holds .git or jevgate.toml.
root=$PWD
while [ ! -e "$root/.git" ] && [ ! -f "$root/jevgate.toml" ] && [ "$root" != / ]; do
    root=$(dirname "$root")
done
report="$root/.jevgate/latest.json"

work=$(mktemp -d) || exit 2
trap 'rm -rf "$work"' EXIT

# Run one check, print its headline (status, gate, files, API requests,
# input tokens, cost) and keep its report as $work/$1.json.
check() {
    name=$1
    shift
    jevgate check "$@" >"$work/$name.out" 2>"$work/$name.err"
    status=$?
    head -n 1 "$work/$name.out"
    if [ "$status" -gt 1 ]; then
        echo "The $name check did not finish (exit $status), so there is nothing to compare." >&2
        tail -n +2 "$work/$name.out" >&2
        cat "$work/$name.err" >&2
        # The default output counts failed files; the report says why.
        jq -r 'first(.errors[], (.files[] | select(.status == "error") | .error)) // empty
            | "First error: \(.)"' "$report" >&2 2>/dev/null
        exit 2
    fi
    cp "$report" "$work/$name.json" || exit 2
}

check first "$@"
check rerun "$@"

# What a run found: whether it completed, its gate, and each file's
# status, findings and raw answers. Timings, costs and cache flags differ
# between the runs by design and are left out.
found='{complete, errors, gate, files: [.files[] | {path, status, error, findings, judgments}]}'
jq -S "$found" "$work/first.json" >"$work/first.found" || exit 2
jq -S "$found" "$work/rerun.json" >"$work/rerun.found" || exit 2
requests=$(jq -e '.api_requests' "$work/rerun.json") || exit 2
case $requests in
    0) sent="no request" ;;
    1) sent="1 request" ;;
    *) sent="$requests requests" ;;
esac

if cmp -s "$work/first.found" "$work/rerun.found"; then
    if [ "$requests" -eq 0 ]; then
        jq -r '
            def count(n; noun): "\(n) \(noun)\(if n == 1 then "" else "s" end)";
            [.files[].findings // [] | .[]] as $findings
            | (["review", "consider", "note"]
                | map(. as $level | [$findings[] | select(.strength == $level)] | length as $n
                    | select($n > 0) | count($n; $level))
                | join(", ")) as $levels
            | "The rerun sent no request and matched: \(count($findings | length; "finding"))"
              + (if $levels == "" then "" else " (\($levels))" end)
              + " and \(count([.files[].judgments // [] | .[]] | length; "answer")) in \(count(.files | length; "file")), with the same levels, lines and probabilities."
        ' "$work/rerun.json" || exit 2
        exit 0
    fi
    echo "The rerun matched, but sent $sent the cache did not answer."
    exit 1
fi

echo "The rerun sent $sent, and its results differ:"
jq -r --slurpfile first "$work/first.json" '
    def by_path: [.files[] | {key: .path, value: {status, error, findings, judgments}}] | from_entries;
    def run: {complete, errors, gate};
    ($first[0] | by_path) as $a | by_path as $b
    | (if ($first[0] | run) != run then "- the run: its completion, errors or gate" else empty end),
      (($a + $b | keys[]) as $path
       | select($a[$path] != $b[$path])
       | "- \($path): " + ([
           (if $a[$path].status != $b[$path].status then "status \($a[$path].status) then \($b[$path].status)" else empty end),
           (if $a[$path].findings != $b[$path].findings then "findings" else empty end),
           (if $a[$path].judgments != $b[$path].judgments then "answers" else empty end)
         ] | join(", ")))' "$work/rerun.json"
exit 1
