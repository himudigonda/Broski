#!/bin/bash

# Ignore generated/build/cache folders, binary archives, media assets, and logs.
# Includes .env.* and .vercel for security hygiene.
IGNORE_PATTERN="(target|cache|dist|node_modules|test|build|artifacts|\.docusaurus|\.broski|\.git|\.lock|package-lock\.json|\.zip|\.tar|\.gz|\.pyc|\.o|\.a|\.d|\.bin|\.tag|\.log$|\.tmp$|\.map$|\.svg$|\.png$|\.jpg$|\.jpeg$|\.gif$|\.webp$|\.ico$|\.pdf$|\.DS_Store$|\.env.*|\.vercel)"

# Dump contents
find . -type f | grep -vE "$IGNORE_PATTERN" | sort | while read -r file; do
    # Skip binaries
    if file --mime "$file" | grep -q 'charset=binary'; then continue; fi
    
    echo "========================================"
    echo "FILE: $file"
    echo "========================================"
    cat "$file"
    echo -e "\n"
done > /tmp/broski_dump.txt

cat /tmp/broski_dump.txt

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
echo "TOTALS: $(wc -l < /tmp/broski_dump.txt | tr -d ' ') Lines, $(wc -w < /tmp/broski_dump.txt | tr -d ' ') Words, $(wc -m < /tmp/broski_dump.txt | tr -d ' ') Chars"
echo "====================================================================="

rm -f /tmp/broski_dump.txt
