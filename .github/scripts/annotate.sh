#!/bin/sh
# Run a cargo command and turn its errors into GitHub annotations, so they
# show on the pull request without opening the log.
out=$(mktemp)
"$@" > "$out" 2>&1
status=$?
cat "$out"
grep -E '^[^ ]+\.rs:[0-9]+:[0-9]+: (error|warning)' "$out" | head -n 40 | while IFS= read -r line; do
    file=${line%%:*}
    rest=${line#*:}
    printf '::error file=%s,line=%s::%s\n' "$file" "${rest%%:*}" "$line"
done
grep -E 'panicked at [^ ]+\.rs:[0-9]+' "$out" | head -n 20 | while IFS= read -r line; do
    loc=$(printf '%s\n' "$line" | sed -E 's/.*panicked at ([^ :]+\.rs):([0-9]+).*/\1 \2/')
    printf '::error file=%s,line=%s::%s\n' "${loc% *}" "${loc#* }" "$line"
done
# Errors without a location (e.g. a failed build script) still get one line.
if [ "$status" -ne 0 ]; then
    grep -E '^error' "$out" | head -n 5 | while IFS= read -r line; do
        printf '::error::%s\n' "$line"
    done
fi
rm -f "$out"
exit "$status"
