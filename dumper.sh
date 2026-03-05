#!/bin/bash

# Ignore generated/build/cache folders, binary archives, media assets, and logs.
# Keep key source files, including website and Rust config/source files.
IGNORE_PATTERN="(target|cache|dist|node_modules|build|artifacts|\.docusaurus|\.please|\.git|\.lock|package-lock\.json|\.zip|\.tar|\.gz|\.pyc|\.o|\.a|\.d|\.bin|\.tag|\.log$|\.tmp$|\.map$|\.svg$|\.png$|\.jpg$|\.jpeg$|\.gif$|\.webp$|\.ico$|\.pdf$|\.DS_Store$)"

# Dump contents
find . -type f | grep -vE "$IGNORE_PATTERN" | sort | while read -r file; do
    # Skip binaries
    if file --mime "$file" | grep -q 'charset=binary'; then continue; fi
    
    echo "========================================"
    echo "FILE: $file"
    echo "========================================"
    cat "$file"
    echo -e "\n"
done > /tmp/please_dump.txt

cat /tmp/please_dump.txt

# Calculate stats
echo "====================================================================="
echo " FILE-WISE BREAKDOWN"
echo "====================================================================="
printf "%-50s | %-8s | %-8s | %-8s\n" "FILE" "LINES" "WORDS" "CHARS"
echo "---------------------------------------------------------------------"

find . -type f | grep -vE "$IGNORE_PATTERN" | sort | while read -r file; do
    if file --mime "$file" | grep -q 'charset=binary'; then continue; fi
    
    stats=$(wc -lwc < "$file")
    printf "%-50s | %-8s | %-8s | %-8s\n" "$file" $(echo $stats)
done

echo "---------------------------------------------------------------------"
echo "TOTALS: $(wc -l < /tmp/please_dump.txt | tr -d ' ') Lines, $(wc -w < /tmp/please_dump.txt | tr -d ' ') Words, $(wc -m < /tmp/please_dump.txt | tr -d ' ') Chars"
echo "====================================================================="

rm -f /tmp/please_dump.txt
