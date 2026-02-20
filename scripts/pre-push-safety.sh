#!/usr/bin/env bash
# =============================================================================
# Pre-push safety hook for the Knol monorepo.
#
# Prevents accidental pushes of the full monorepo (knol-platform) to the
# public OSS remote (oss-public / aiknol/knol). Only subtree pushes of
# the knol-oss/ directory should go to the public repo.
#
# Install:
#   cp scripts/pre-push-safety.sh .git/hooks/pre-push
#   chmod +x .git/hooks/pre-push
# =============================================================================

remote="$1"
url="$2"

# Detect if we're pushing to the public OSS repo
if echo "$url" | grep -qE "aiknol/knol(\.git)?$"; then
    # Allow subtree pushes (git subtree push uses a temporary branch)
    # Block direct branch pushes from the full monorepo
    while read local_ref local_sha remote_ref remote_sha; do
        # Subtree push creates refs like refs/heads/... with synthetic commits
        # Direct push would push main/dev branches which contain proprietary code
        branch=$(echo "$local_ref" | sed 's|refs/heads/||')
        if [ "$branch" = "main" ] || [ "$branch" = "dev" ] || [ "$branch" = "master" ]; then
            echo ""
            echo "ERROR: Blocked direct push of '$branch' to public repo ($url)."
            echo ""
            echo "  The '$branch' branch contains proprietary code from knol-platform."
            echo "  To push only the OSS code, use:"
            echo ""
            echo "    git subtree push --prefix=knol-oss oss-public main"
            echo ""
            exit 1
        fi
    done
fi

exit 0
