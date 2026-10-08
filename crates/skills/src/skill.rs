use anyhow::bail;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillMeta {
    pub name: String,
    pub description: String,
    pub when_to_use: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Skill {
    pub meta: SkillMeta,
    pub body: String,
}

/// Parse a SKILL.md file: `---` delimited frontmatter of `key: value` lines, then markdown.
/// Values may be quoted. Unknown keys are allowed (the Claude CLI understands more).
pub fn parse_skill(text: &str) -> crate::Result<Skill> {
    let text = text.trim_start_matches('\u{feff}');
    let Some(rest) = text.strip_prefix("---\n").or_else(|| text.strip_prefix("---\r\n")) else {
        bail!("SKILL.md must start with '---' frontmatter");
    };
    let Some(end) = rest.find("\n---") else { bail!("unterminated frontmatter") };
    let (front, body) = (&rest[..end], &rest[end + 4..]);
    let body = body.trim_start_matches(['\r', '\n']).to_string();
    let (mut name, mut description, mut when) = (None, None, None);
    for line in front.lines() {
        let Some((k, v)) = line.split_once(':') else { continue };
        let v = unquote(v.trim());
        match k.trim() {
            "name" => name = Some(v),
            "description" => description = Some(v),
            "when_to_use" | "when-to-use" => when = Some(v),
            _ => {}
        }
    }
    let (Some(name), Some(description)) = (name, description) else {
        bail!("frontmatter needs `name` and `description`");
    };
    if description.trim().is_empty() {
        bail!("description must not be empty");
    }
    Ok(Skill { meta: SkillMeta { name, description, when_to_use: when.filter(|w| !w.is_empty()) }, body })
}

pub fn render_skill(skill: &Skill) -> String {
    let mut s = format!("---\nname: {}\ndescription: {}\n", skill.meta.name, quote(&skill.meta.description));
    if let Some(w) = &skill.meta.when_to_use {
        s.push_str(&format!("when_to_use: {}\n", quote(w)));
    }
    s.push_str("---\n\n");
    s.push_str(&skill.body);
    s
}

fn unquote(v: &str) -> String {
    if v.len() >= 2 && ((v.starts_with('"') && v.ends_with('"')) || (v.starts_with('\'') && v.ends_with('\''))) {
        v[1..v.len() - 1].replace("\\\"", "\"")
    } else {
        v.to_string()
    }
}

fn quote(v: &str) -> String {
    let v = v.replace('\n', " ");
    if v.contains(':') || v.contains('#') || v.starts_with(['"', '\'', ' ']) {
        format!("\"{}\"", v.replace('"', "\\\""))
    } else {
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let s = parse_skill("---\nname: run-tests\ndescription: \"How to run tests: use cargo\"\nwhen_to_use: Before committing\nallowed-tools: Bash\n---\n\n# Steps\n1. cargo test\n").unwrap();
        assert_eq!(s.meta.name, "run-tests");
        assert_eq!(s.meta.description, "How to run tests: use cargo");
        assert_eq!(s.meta.when_to_use.as_deref(), Some("Before committing"));
        assert_eq!(s.body, "# Steps\n1. cargo test\n");
        assert_eq!(parse_skill(&render_skill(&s)).unwrap(), s);
        assert!(parse_skill("# no frontmatter").is_err());
        assert!(parse_skill("---\nname: x\n---\nbody").is_err());
    }
}
