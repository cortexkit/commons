//! Tool capability names, kept local so role contracts remain independent at runtime.

// These unprefixed names belong to the tool-provider role. Agreement tests
// compare both roles' names and grammar without coupling their runtime releases.
const DEFINED_TOOL_CAPABILITIES: &[&str] = &[
    "shell.exec/v1",
    "code.read/v1",
    "code.edit/v1",
    "code.search/v1",
    "code.files/v1",
    "code.outline/v1",
    "code.callgraph/v1",
    "code.diagnostics/v1",
    "browser.use/v1",
    "computer.use/v1",
    "git.history/v1",
    "web.search/v1",
    "github.read/v1",
    "code.search.public/v1",
    "tool.view/v1",
    "llm.classify/v1",
    "llm.extract/v1",
    "llm.complete/v1",
];

pub(crate) fn is_capability_name(tag: &str) -> bool {
    let Some((namespace, name)) = tag.split_once(':') else {
        return DEFINED_TOOL_CAPABILITIES.contains(&tag);
    };
    let namespace_ok = namespace.split('-').all(|word| {
        !word.is_empty()
            && word
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
    });
    let Some((name, version)) = name.rsplit_once("/v") else {
        return false;
    };
    let name_ok = name.split('.').all(|word| {
        !word.is_empty()
            && word
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    });
    let version_ok = !version.is_empty()
        && !version.starts_with('0')
        && version.bytes().all(|byte| byte.is_ascii_digit());
    namespace_ok && name_ok && version_ok
}

#[cfg(test)]
mod tests {
    use super::*;
    use cortexkit_role_tool_provider::{check_capability_tag, DEFINED_CAPABILITY_TAGS};

    #[test]
    fn capability_name_validator_agrees_with_tool_provider() {
        let cases = [
            ("example:code.read/v1", true),
            ("example:code.read/v12", true),
            ("example-tools:code.read_write/v2", true),
            ("0:0/v1", true),
            ("example:_/v1", true),
            ("example:code/v18446744073709551616", true),
            ("", false),
            ("unknown/v1", false),
            ("code.read/v2", false),
            (":code.read/v1", false),
            (":", false),
            ("-example:code.read/v1", false),
            ("example-:code.read/v1", false),
            ("example--tools:code.read/v1", false),
            ("Example:code.read/v1", false),
            ("example_tools:code.read/v1", false),
            ("example.tools:code.read/v1", false),
            ("example tools:code.read/v1", false),
            ("é:code.read/v1", false),
            ("example:", false),
            ("example:code.read", false),
            ("example:/v1", false),
            ("example:.code/v1", false),
            ("example:code./v1", false),
            ("example:code..read/v1", false),
            ("example:Code.read/v1", false),
            ("example:code-read/v1", false),
            ("example:code read/v1", false),
            ("example:é/v1", false),
            ("example:code:read/v1", false),
            ("example:code/read/v1", false),
            ("example:code/v1/v2", false),
            ("example:code/v", false),
            ("example:code/v0", false),
            ("example:code/v01", false),
            ("example:code/v-1", false),
            ("example:code/v+1", false),
            ("example:code/v1.0", false),
            ("example:code/v1a", false),
            ("example:code/v١", false),
            ("example:code/v1\n", false),
        ];
        for (tag, expected) in cases {
            assert_agreement(tag, expected);
        }
        for tag in DEFINED_TOOL_CAPABILITIES
            .iter()
            .chain(DEFINED_CAPABILITY_TAGS)
        {
            assert_agreement(tag, true);
        }
        // Capability names have no length cap, unlike runner API/auth tokens.
        // Check both component and total lengths around common token limits.
        for length in [1, 63, 64, 65, 255, 256, 257, 1024] {
            assert_agreement(&format!("{}:read/v1", "a".repeat(length)), true);
            assert_agreement(&format!("example:{}/v1", "a".repeat(length)), true);
            assert_agreement(&format!("example:read/v{}", "9".repeat(length)), true);
        }
        for length in [63, 64, 65] {
            assert_agreement(&format!("a:{}/v1", "a".repeat(length - 5)), true);
        }
    }

    fn assert_agreement(tag: &str, expected: bool) {
        let local = is_capability_name(tag);
        let upstream = check_capability_tag(tag).is_ok();
        assert_eq!(local, upstream, "capability grammar drift for {tag:?}");
        assert_eq!(local, expected, "unexpected capability verdict for {tag:?}");
    }
}
