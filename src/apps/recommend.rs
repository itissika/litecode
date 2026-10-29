//! External app recommendations: metadata only.
//!
//! A recommendation is not a registry entry. LiteCode cannot install, update,
//! or uninstall it, so it never runs from here. The panel shows it under
//! "not installed" and offers a message for the agent, which can install the
//! runtime and register a launch definition that points at real paths.

use std::collections::BTreeMap;

use super::schema::AppCategory;

#[derive(Debug, Clone)]
pub struct Recommendation {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub description_i18n: BTreeMap<String, String>,
    pub category: AppCategory,
    pub icon: &'static str,
    pub repo: &'static str,
}

fn rec(
    id: &'static str,
    name: &'static str,
    description: &'static str,
    zh: &'static str,
    category: AppCategory,
    icon: &'static str,
    repo: &'static str,
) -> Recommendation {
    Recommendation {
        id,
        name,
        description,
        description_i18n: BTreeMap::from([("zh-CN".to_string(), zh.to_string())]),
        category,
        icon,
        repo,
    }
}

/// Apps LiteCode deliberately does not manage: they need a user-supplied
/// runtime or a system install. An entry disappears from this list as soon as
/// the user registers an app with the same id.
pub fn recommendations() -> Vec<Recommendation> {
    vec![
        rec(
            "comfyui",
            "ComfyUI",
            "Node-based Stable Diffusion workflow editor.",
            "节点式 Stable Diffusion 工作流编辑器。",
            AppCategory::Image,
            "🧩",
            "https://github.com/comfyanonymous/ComfyUI",
        ),
        rec(
            "excalidraw",
            "Excalidraw",
            "Hand-drawn style whiteboard and diagrams.",
            "手绘风格的白板与示意图。",
            AppCategory::Design,
            "✏️",
            "https://github.com/excalidraw/excalidraw",
        ),
        rec(
            "rawgraphs",
            "RAWGraphs",
            "Open-source visualization framework for tabular data.",
            "面向表格数据的开源可视化框架。",
            AppCategory::Analysis,
            "📊",
            "https://github.com/rawgraphs/rawgraphs-app",
        ),
        rec(
            "quarto",
            "Quarto",
            "Scientific and technical publishing from Markdown.",
            "从 Markdown 发布科技与学术内容。",
            AppCategory::Document,
            "📄",
            "https://github.com/quarto-dev/quarto-cli",
        ),
        rec(
            "vikunja",
            "Vikunja",
            "Self-hosted task and project management.",
            "自托管的任务与项目管理。",
            AppCategory::Task,
            "✅",
            "https://github.com/go-vikunja/vikunja",
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recommendations_are_unique_and_link_to_https_repos() {
        let all = recommendations();
        let ids: std::collections::BTreeSet<_> = all.iter().map(|app| app.id).collect();
        assert_eq!(ids.len(), all.len());
        for app in all {
            assert!(app.repo.starts_with("https://"), "{}", app.repo);
            assert!(!app.description.is_empty());
            assert!(app.description_i18n.contains_key("zh-CN"));
        }
    }
}
