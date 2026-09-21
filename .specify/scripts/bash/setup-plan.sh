#!/usr/bin/env bash

set -e

# Parse command line arguments
JSON_MODE=false

for arg in "$@"; do
    case "$arg" in
        --json)
            JSON_MODE=true
            ;;
        --help|-h)
            echo "Usage: $0 [--json]"
            echo "  --json    Output results in JSON format"
            echo "  --help    Show this help message"
            exit 0
            ;;
        *)
            echo "ERROR: Unknown option '$arg'" >&2
            exit 1
            ;;
    esac
done

# Get script directory and load common functions
SCRIPT_DIR="$(CDPATH="" cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/common.sh"

# Get all paths and variables from common functions
_paths_output=$(get_feature_paths) || { echo "ERROR: Failed to resolve feature paths" >&2; exit 1; }
eval "$_paths_output"
unset _paths_output

# Ensure the feature directory exists
mkdir -p "$FEATURE_DIR"

# Copy plan template if plan doesn't already exist
if [[ -f "$IMPL_PLAN" ]]; then
    if $JSON_MODE; then
        echo "Plan already exists at $IMPL_PLAN, skipping template copy" >&2
    else
        echo "Plan already exists at $IMPL_PLAN, skipping template copy"
    fi
else
    if resolve_template_content "plan-template" "$REPO_ROOT" > "$IMPL_PLAN"; then
        if $JSON_MODE; then
            echo "Copied plan template to $IMPL_PLAN" >&2
        else
            echo "Copied plan template to $IMPL_PLAN"
        fi
    else
        resolve_status=$?
        rm -f "$IMPL_PLAN"
        if [ "$resolve_status" -ne 1 ]; then
            exit "$resolve_status"
        fi
        if $JSON_MODE; then
            echo "Warning: Plan template not found" >&2
        else
            echo "Warning: Plan template not found"
        fi
        touch "$IMPL_PLAN"
    fi
fi

# Resolve the Phase 2 design templates through the override stack.
#
# Two-tier failure handling, matching the plan-template block above: a template
# that is simply absent degrades to an empty value so a project with stale shared
# templates can still plan, while a template that exists but cannot be composed
# propagates the resolver's status. The plan command treats an empty value as a
# blocking error rather than inventing a section structure.
DESIGN_TEMPLATE_RESULT=""
resolve_design_template() {
    local name="$1" content resolve_status
    DESIGN_TEMPLATE_RESULT=""
    # The status must be captured in an explicit else branch: after a failed `if`
    # with no else, bash sets $? to 0, which would swallow both the warning and
    # the fatal tier.
    if content=$(resolve_template_content "$name" "$REPO_ROOT"; resolve_status=$?; printf x; exit "$resolve_status"); then
        DESIGN_TEMPLATE_RESULT="${content%x}"
        return 0
    else
        resolve_status=$?
        if [ "$resolve_status" -ne 1 ]; then
            return "$resolve_status"
        fi
        # Diagnostic goes to stderr in both modes so text-mode stdout stays a
        # clean report of paths.
        echo "Warning: $name not found" >&2
        return 0
    fi
}

resolve_design_template "architecture-template" || exit $?
ARCHITECTURE_TEMPLATE_CONTENT="$DESIGN_TEMPLATE_RESULT"
resolve_design_template "design-template" || exit $?
DESIGN_TEMPLATE_CONTENT="$DESIGN_TEMPLATE_RESULT"

# Output results
if $JSON_MODE; then
    if has_jq; then
        jq -cn \
            --arg feature_spec "$FEATURE_SPEC" \
            --arg impl_plan "$IMPL_PLAN" \
            --arg feature_dir "$FEATURE_DIR" \
            --arg branch "$CURRENT_BRANCH" \
            --arg architecture_template_content "$ARCHITECTURE_TEMPLATE_CONTENT" \
            --arg design_template_content "$DESIGN_TEMPLATE_CONTENT" \
            '{FEATURE_SPEC:$feature_spec,IMPL_PLAN:$impl_plan,FEATURE_DIR:$feature_dir,BRANCH:$branch,ARCHITECTURE_TEMPLATE_CONTENT:$architecture_template_content,DESIGN_TEMPLATE_CONTENT:$design_template_content}'
    else
        printf '{"FEATURE_SPEC":"%s","IMPL_PLAN":"%s","FEATURE_DIR":"%s","BRANCH":"%s","ARCHITECTURE_TEMPLATE_CONTENT":"%s","DESIGN_TEMPLATE_CONTENT":"%s"}\n' \
            "$(json_escape "$FEATURE_SPEC")" "$(json_escape "$IMPL_PLAN")" "$(json_escape "$FEATURE_DIR")" "$(json_escape "$CURRENT_BRANCH")" "$(json_escape "$ARCHITECTURE_TEMPLATE_CONTENT")" "$(json_escape "$DESIGN_TEMPLATE_CONTENT")"
    fi
else
    echo "FEATURE_SPEC: $FEATURE_SPEC"
    echo "IMPL_PLAN: $IMPL_PLAN"
    echo "FEATURE_DIR: $FEATURE_DIR"
    echo "BRANCH: $CURRENT_BRANCH"
fi
