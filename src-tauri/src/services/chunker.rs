use crate::models::note::Note;

// Ignore tiny fragments that are usually not useful for question generation.
const MIN_CHUNK_CHARS: usize = 200;
// Keep chunk size bounded so prompts remain focused and inexpensive.
const MAX_CHUNK_CHARS: usize = 1500;

#[derive(Debug, Clone)]
pub struct MarkdownChunk {
    // Original note path, used for traceability and dedup later.
    pub note_path: String,
    // Note title derived from the file name.
    pub note_title: String,
    // Section heading this chunk belongs to.
    pub heading: String,
    // Actual text content sent to downstream generation.
    pub content: String,
    // 1-based source line numbers from the original note.
    pub start_line: usize,
    pub end_line: usize,
    // Position metadata to keep chunk order stable.
    pub section_index: usize,
    pub chunk_index: usize,
}

#[derive(Debug, Clone)]
struct Section {
    heading: String,
    content: String,
    start_line: usize,
    end_line: usize,
}

/// Convenience wrapper that chunks a full `Note` model.
pub fn chunk_note(note: &Note) -> Vec<MarkdownChunk> {
    chunk_markdown(&note.path, &note.title, &note.content)
}

/// Splits markdown into heading-based chunks with size guardrails.
///
/// Strategy:
/// - Split into logical sections by preferred heading level (H1 first, then H2).
/// - Refine oversized sections by cascading through sub-heading levels (H2→H3→...→H6).
/// - Fall back to paragraph-level splitting when no deeper headings exist.
/// - Merge tiny chunks below `MIN_CHUNK_CHARS` into neighboring chunks.
/// - If everything is filtered out, return one fallback chunk for the full note.
pub fn chunk_markdown(note_path: &str, note_title: &str, markdown: &str) -> Vec<MarkdownChunk> {
    // Step 1: Split at the top-level heading (H1 preferred).
    let sections = split_into_sections(note_title, markdown);

    // Determine top-level heading for cascading refinement.
    let preferred_level = {
        let lines: Vec<&str> = markdown.lines().collect();
        find_preferred_heading_level(&lines).unwrap_or(0)
    };

    // Step 2: Cascade oversized sections through deeper heading levels.
    let refined = refine_oversized_sections(sections, preferred_level);

    // Step 2.5: Merge tiny boundary sections so metadata-only/introduction
    // fragments do not become standalone chunks.
    let refined = merge_tiny_sections(refined, MIN_CHUNK_CHARS);

    // Step 3: Paragraph-level splitting + small-chunk merging on each section.
    let mut chunks = Vec::new();

    for (section_index, section) in refined.into_iter().enumerate() {
        let chunk_contents =
            merge_small_chunks(split_large_content(&section.content, MAX_CHUNK_CHARS), MIN_CHUNK_CHARS);

        for (chunk_index, chunk_content) in chunk_contents.into_iter().enumerate() {
            let trimmed = chunk_content.trim();
            if trimmed.is_empty() {
                continue;
            }

            chunks.push(MarkdownChunk {
                note_path: note_path.to_string(),
                note_title: note_title.to_string(),
                heading: section.heading.clone(),
                content: trimmed.to_string(),
                start_line: section.start_line,
                end_line: section.end_line,
                section_index,
                chunk_index,
            });
        }
    }

    // Fallback: keep at least one chunk when the note has content.
    if chunks.is_empty() {
        let fallback = markdown.trim();
        if !fallback.is_empty() {
            chunks.push(MarkdownChunk {
                note_path: note_path.to_string(),
                note_title: note_title.to_string(),
                heading: note_title.to_string(),
                content: fallback.to_string(),
                start_line: 1,
                end_line: markdown.lines().count().max(1),
                section_index: 0,
                chunk_index: 0,
            });
        }
    }

    chunks
}

/// Merges tiny sections into neighboring sections before chunk splitting.
///
/// Rules mirror chunk-level merging:
/// - Prefer appending tiny sections to the previous section.
/// - If there is no previous section, prepend tiny section content to the next section.
/// - If no neighbor exists, keep the section as-is.
fn merge_tiny_sections(mut sections: Vec<Section>, min_chars: usize) -> Vec<Section> {
    // Fast path: nothing to merge when there is only one section or merging is disabled.
    if sections.len() <= 1 || min_chars == 0 {
        return sections;
    }

    let mut index = 0;
    while index < sections.len() {
        // Only sections below the configured threshold are candidates for merging.
        let is_tiny = sections[index].content.trim().len() < min_chars;
        if !is_tiny {
            index += 1;
            continue;
        }

        // Prefer merging tiny sections into the previous section to keep forward flow stable.
        if index > 0 {
            let tiny = sections.remove(index);
            let previous = &mut sections[index - 1];
            previous.content.push_str("\n\n");
            previous.content.push_str(tiny.content.trim());
            previous.end_line = tiny.end_line;
            continue;
        }

        // If there is no previous section, merge into the next section instead.
        if sections.len() > 1 {
            let tiny = sections.remove(index);
            let next = &mut sections[index];
            next.content = format!("{}\n\n{}", tiny.content.trim(), next.content.trim());
            next.start_line = tiny.start_line;
            continue;
        }

        // Isolated tiny section with no neighbors: keep it unchanged.
        index += 1;
    }

    sections
}

/// Merges tiny chunk fragments into adjacent chunks.
///
/// Rules:
/// - Prefer appending tiny fragments to the previous chunk.
/// - If there is no previous chunk, prepend them to the next chunk.
/// - If no neighbor exists, keep the tiny fragment as its own chunk.
fn merge_small_chunks(raw_chunks: Vec<String>, min_chars: usize) -> Vec<String> {
    let mut pending = raw_chunks
        .into_iter()
        .map(|chunk| chunk.trim().to_string())
        .filter(|chunk| !chunk.is_empty())
        .collect::<Vec<_>>();

    if pending.len() <= 1 || min_chars == 0 {
        return pending;
    }

    let mut merged = Vec::new();

    for index in 0..pending.len() {
        let current = pending[index].clone();

        // If current chunk length is at least min_chars, it is pushed directly to output.
        if current.len() >= min_chars {
            merged.push(current);
            continue;
        }
        
        // If output already has a previous chunk, append current to that previous chunk.
        if let Some(previous) = merged.last_mut() {
            previous.push_str("\n\n");
            previous.push_str(&current);
            continue;
        }
        
        // Else if there is a next pending chunk, prepend current into that next chunk.
        if index + 1 < pending.len() {
            let next = pending[index + 1].clone();
            pending[index + 1] = format!("{}\n\n{}", current, next);
            continue;
        }

        merged.push(current);
    }

    merged
}

/// Recursively refines oversized sections by splitting on deeper heading levels.
///
/// For each section exceeding MAX_CHUNK_CHARS:
/// - Try splitting by heading level parent_level+1, then +2, up to H6.
/// - If sub-headings are found, recursively refine the resulting sub-sections.
/// - If no sub-headings at any level, keep the section for paragraph-level splitting later.
fn refine_oversized_sections(sections: Vec<Section>, parent_level: usize) -> Vec<Section> {
    let mut result = Vec::new();

    for section in sections {
        // Section fits within limit — keep as-is.
        if section.content.trim().len() <= MAX_CHUNK_CHARS {
            result.push(section);
            continue;
        }

        // Try each deeper heading level before falling back to paragraph split.
        let mut split_found = false;
        for try_level in (parent_level + 1)..=6 {
            let sub_sections = split_at_level(&section.heading, &section.content, try_level);
            if sub_sections.len() > 1 {
                // Recursively refine sub-sections that may still be oversized.
                let refined = refine_oversized_sections(sub_sections, try_level);
                result.extend(refined);
                split_found = true;
                break;
            }
        }

        if !split_found {
            // No sub-headings at any deeper level; keep for paragraph-level splitting.
            result.push(section);
        }
    }

    result
}

/// Splits markdown into semantic sections using the preferred heading level.
///
/// Preference order is H1 first, then H2. If no headings exist, the whole
/// note is returned as a single section.
fn split_into_sections(note_title: &str, markdown: &str) -> Vec<Section> {
    let lines: Vec<&str> = markdown.lines().collect();
    if lines.is_empty() {
        return Vec::new();
    }

    match find_preferred_heading_level(&lines) {
        Some(level) => split_at_level(note_title, markdown, level),
        None => vec![Section {
            heading: note_title.to_string(),
            content: markdown.to_string(),
            start_line: 1,
            end_line: lines.len(),
        }],
    }
}

/// Splits content at a specific heading level.
///
/// Lines before the first matching heading become a section with `fallback_heading`.
/// Each matching heading starts a new section. Content between headings is preserved.
/// Ignores headings inside fenced code blocks.
fn split_at_level(fallback_heading: &str, content: &str, level: usize) -> Vec<Section> {
    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() {
        return Vec::new();
    }

    let mut sections = Vec::new();
    let mut current_heading = fallback_heading.to_string();
    let mut current_start = 1;
    let mut current_lines: Vec<&str> = Vec::new();
    let mut in_code_block = false;

    for (idx, line) in lines.iter().enumerate() {
        let line_no = idx + 1;
        let trimmed = line.trim();

        // Toggle fenced code block mode so markdown headings inside code are ignored.
        if trimmed.starts_with("```") {
            in_code_block = !in_code_block;
        }

        if !in_code_block {
            if let Some((heading_level, heading_text)) = parse_heading(trimmed) {
                if heading_level == level {
                    if !current_lines.is_empty() {
                        sections.push(Section {
                            heading: current_heading.clone(),
                            content: current_lines.join("\n"),
                            start_line: current_start,
                            end_line: line_no.saturating_sub(1).max(current_start),
                        });
                    }

                    // Start a new section at this heading line.
                    current_heading = if heading_text.is_empty() {
                        fallback_heading.to_string()
                    } else {
                        heading_text.to_string()
                    };
                    current_start = line_no;
                    current_lines.clear();
                    continue;
                }
            }
        }

        current_lines.push(line);
    }

    // Push final buffered section.
    if !current_lines.is_empty() {
        sections.push(Section {
            heading: current_heading,
            content: current_lines.join("\n"),
            start_line: current_start,
            end_line: lines.len(),
        });
    }

    sections
        .into_iter()
        .filter(|section| !section.content.trim().is_empty())
        .collect()
}

/// Detects which heading level to use as the primary split point.
///
/// Prefers H1 first so top-level structure is preserved, then falls back to H2.
fn find_preferred_heading_level(lines: &[&str]) -> Option<usize> {
    let mut has_h1 = false;
    let mut has_h2 = false;
    let mut in_code_block = false;

    for line in lines {
        let trimmed = line.trim();

        if trimmed.starts_with("```") {
            in_code_block = !in_code_block;
        }

        if in_code_block {
            continue;
        }

        if let Some((level, _)) = parse_heading(trimmed) {
            if level == 1 {
                has_h1 = true;
            }
            if level == 2 {
                has_h2 = true;
            }
        }
    }

    if has_h1 {
        return Some(1);
    }

    if has_h2 {
        return Some(2);
    }

    None
}

/// Parses an ATX markdown heading (`#`..`######`) and returns `(level, text)`.
fn parse_heading(line: &str) -> Option<(usize, &str)> {
    let hashes = line.chars().take_while(|ch| *ch == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }

    let rest = line.get(hashes..)?.trim_start();
    if rest.is_empty() {
        return None;
    }

    Some((hashes, rest))
}

/// Splits one section's content into chunks at paragraph boundaries.
///
/// Paragraph-first splitting preserves readability. If a single paragraph is
/// still too large, we hard-split it by character limit.
fn split_large_content(content: &str, max_chars: usize) -> Vec<String> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    if trimmed.len() <= max_chars {
        return vec![trimmed.to_string()];
    }

    let paragraphs = split_paragraphs(trimmed);
    let mut chunks = Vec::new();
    let mut current = String::new();

    for paragraph in paragraphs {
        if paragraph.len() > max_chars {
            if !current.trim().is_empty() {
                chunks.push(current.trim().to_string());
                current.clear();
            }

            chunks.extend(split_hard_limit(&paragraph, max_chars));
            continue;
        }

        // Account for two newline characters between paragraphs.
        let separator = if current.is_empty() { 0 } else { 2 };
        if current.len() + separator + paragraph.len() > max_chars {
            if !current.trim().is_empty() {
                chunks.push(current.trim().to_string());
            }
            current = paragraph;
        } else {
            if !current.is_empty() {
                current.push_str("\n\n");
            }
            current.push_str(&paragraph);
        }
    }

    if !current.trim().is_empty() {
        chunks.push(current.trim().to_string());
    }

    chunks
}

/// Splits content into paragraphs using blank lines as separators.
fn split_paragraphs(content: &str) -> Vec<String> {
    let mut paragraphs = Vec::new();
    let mut current = Vec::new();

    for line in content.lines() {
        if line.trim().is_empty() {
            if !current.is_empty() {
                paragraphs.push(current.join("\n").trim().to_string());
                current.clear();
            }
            continue;
        }

        current.push(line.to_string());
    }

    if !current.is_empty() {
        paragraphs.push(current.join("\n").trim().to_string());
    }

    paragraphs
}

/// Hard-splits a long string into fixed-size character windows.
///
/// This is a fallback for pathological paragraphs that exceed `max_chars`.
fn split_hard_limit(text: &str, max_chars: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut start = 0;
    let chars: Vec<char> = text.chars().collect();

    while start < chars.len() {
        let end = (start + max_chars).min(chars.len());
        let chunk: String = chars[start..end].iter().collect();
        out.push(chunk.trim().to_string());
        start = end;
    }

    out.into_iter().filter(|piece| !piece.is_empty()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_h1_as_primary_split() {
        let markdown = "# Section 1\ncontent 1\n\n# Section 2\ncontent 2";
        let sections = split_into_sections("Note", markdown);

        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].heading, "Section 1");
        assert_eq!(sections[1].heading, "Section 2");
    }

    #[test]
    fn splits_by_h2_when_no_h1() {
        let markdown = "## Ownership\nline 1\nline 2\n\n## Borrowing\nline 3\nline 4";
        let sections = split_into_sections("Rust", markdown);

        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].heading, "Ownership");
        assert_eq!(sections[1].heading, "Borrowing");
    }

    #[test]
    fn splits_at_specific_level() {
        let markdown = "intro\n\n## Sub A\nline 1\n\n## Sub B\nline 2";
        let sections = split_at_level("Fallback", markdown, 2);

        assert_eq!(sections.len(), 3);
        assert_eq!(sections[0].heading, "Fallback");
        assert_eq!(sections[1].heading, "Sub A");
        assert_eq!(sections[2].heading, "Sub B");
    }

    #[test]
    fn uses_full_note_when_no_headings() {
        let markdown = "paragraph one\n\nparagraph two";
        let sections = split_into_sections("No Headings", markdown);

        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].heading, "No Headings");
    }

    #[test]
    fn refines_oversized_section_with_sub_headings() {
        let long_text = "x ".repeat(400);
        let markdown = format!("## Sub 1\n{}\n\n## Sub 2\n{}", long_text, long_text);
        let sections = vec![Section {
            heading: String::from("Parent"),
            content: markdown,
            start_line: 1,
            end_line: 10,
        }];
        let refined = refine_oversized_sections(sections, 1);

        assert!(refined.len() >= 2);
        assert_eq!(refined[0].heading, "Sub 1");
        assert_eq!(refined[1].heading, "Sub 2");
    }

    #[test]
    fn h1_with_h2_subsections_preserves_all_content() {
        let markdown = "# Topic\nintro line\n\n## Detail A\ncontent a\n\n## Detail B\ncontent b";
        let sections = split_into_sections("Note", markdown);

        // H1 is preferred, so one top-level section under "Topic"
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].heading, "Topic");
        // All sub-content is inside this section
        assert!(sections[0].content.contains("intro line"));
        assert!(sections[0].content.contains("## Detail A"));
        assert!(sections[0].content.contains("content b"));
    }

    #[test]
    fn tiny_chunk_appends_to_previous_when_possible() {
        let merged = merge_small_chunks(
            vec![
                String::from("This is a reasonably sized chunk."),
                String::from("tiny"),
                String::from("This is another reasonably sized chunk."),
            ],
            10,
        );

        assert_eq!(merged.len(), 2);
        assert!(merged[0].contains("This is a reasonably sized chunk."));
        assert!(merged[0].contains("tiny"));
    }

    #[test]
    fn tiny_first_chunk_prepends_to_next_when_no_previous() {
        let merged = merge_small_chunks(
            vec![
                String::from("tiny"),
                String::from("This is a reasonably sized chunk."),
            ],
            10,
        );

        assert_eq!(merged.len(), 1);
        assert!(merged[0].starts_with("tiny"));
    }

    #[test]
    fn isolated_tiny_chunk_is_kept_when_no_neighbors() {
        let merged = merge_small_chunks(vec![String::from("tiny")], 10);

        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0], "tiny");
    }

    #[test]
    fn tiny_leading_section_is_merged_into_next_section() {
        let note_title = "k8s";
        let intro = "metadata";
        let body = "Kubernetes control plane schedules workloads and keeps desired state. ".repeat(6);
        let markdown = format!(
            "## 05. What is Kubernetes\n{}\n\n## 1. Kubernetes Architecture\n{}",
            intro, body
        );

        let chunks = chunk_markdown("/vault/k8s.md", note_title, &markdown);

        assert!(!chunks.is_empty());
        assert!(chunks[0].content.contains("metadata"));
        assert!(chunks[0].content.contains("Kubernetes control plane"));
        assert_ne!(chunks[0].heading, "05. What is Kubernetes");
    }
}
