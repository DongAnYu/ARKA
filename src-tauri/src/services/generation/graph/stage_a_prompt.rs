// =====================================================================
// Stage A Prompt Formatting
// =====================================================================
//
// Generates the user prompt for Stage A extraction (LLM JSON output formatting).
// Instructs the model to extract knowledge points and their entity references from
// a chunk of markdown, with strict entity consistency rules (closed guest list pattern).

/// Formats the user prompt for Stage A knowledge extraction.
///
/// The prompt instructs the LLM to:
/// 1. Build a complete entity list upfront (closed guest list)
/// 2. Extract knowledge points, each referencing only entities from that list
/// 3. Identify relationships between entities
///
/// Entity consistency rule (core constraint):
/// - Every name appearing in `raw_entity_names` or as a relation `target_entity_name`
///   must also appear in the top-level `entities` array.
/// - Matching is case-insensitive.
/// - No new entity names may surface inside a knowledge point without being
///   declared in the entity list first.
///
/// # Arguments
/// - `chunk_markdown` — The chunk text to extract from
/// - `chunk_index_context` — Contextual information about other chunks (for reference scope)
///
/// # Returns
/// A formatted prompt string ready for LLM input.
pub fn format_stage_a_graph_user_prompt(chunk_markdown: &str, chunk_index_context: &str) -> String {
    format!(
        r#"You are a knowledge graph extraction specialist. Your task is to extract structured knowledge from the following chunk of content and output it as JSON.

## Content to Extract From

```markdown
{}
```

## Chunk Context

For reference, here are other chunks in this document:
{}

## Output Format

You must output a JSON object with exactly this structure:
```json
{{
  "entities": [
    "entity_name_1",
    "entity_name_2"
  ],
  "knowledge_points": [
    {{
      "point": "A clear, standalone fact or claim about one or more entities",
      "knowledge_type": "fact",
      "raw_entity_names": ["entity_name_1", "entity_name_2"],
      "raw_relations": [
        {{
          "target_entity_name": "entity_name_2",
          "relation_type": "related_to",
          "source_quote": "optional direct quote from the text"
        }}
      ]
    }}
  ]
}}
```

## Critical Rule: Closed Entity Guest List

**Every entity name that appears in any knowledge point's `raw_entity_names` or as a `target_entity_name` in `raw_relations` must also appear in the top-level `entities` array.** This is a hard constraint:

- Think of `entities` as a complete "guest list" — every entity that will be mentioned anywhere in the output must be listed there first.
- Matching is case-insensitive (e.g., "chloroplast" and "Chloroplast" refer to the same entity).
- Before writing any knowledge points, identify **every distinct entity** that will be needed anywhere — including entities that will only appear as relation targets, not as a point's main subject.
- **Important:** Include entities mentioned in *contrasts*, *negations*, or *negative comparisons* too. If you write "X is not Y" or "X contrasts with Y," both X and Y must be in the guest list, even if one is mentioned only in the negative or contrastive context.

### Common Mistake to Avoid

A common mistake is mentioning an entity inside a point's `raw_entity_names` or as a relation target without first adding it to the top-level `entities` list. For example:

❌ **WRONG:**
```json
{{
  "entities": ["chloroplast"],
  "knowledge_points": [
    {{
      "point": "Thylakoids are membranes inside chloroplasts",
      "knowledge_type": "fact",
      "raw_entity_names": ["thylakoid", "chloroplast"],
      "raw_relations": []
    }}
  ]
}}
```
The entity "thylakoid" appears in `raw_entity_names` but was never added to `entities`. This violates the guest list rule.

✓ **CORRECT:**
```json
{{
  "entities": ["thylakoid", "chloroplast"],
  "knowledge_points": [
    {{
      "point": "Thylakoids are membranes inside chloroplasts",
      "knowledge_type": "fact",
      "raw_entity_names": ["thylakoid", "chloroplast"],
      "raw_relations": []
    }}
  ]
}}
```
Both "thylakoid" and "chloroplast" are declared in `entities` before being referenced in the point.

## Extraction Process

### Step 1: Enumerate All Entities
Before writing any knowledge points, read through the chunk carefully and identify every distinct entity that should be represented in the knowledge graph. An entity is any:
- Named concept, object, process, or organism
- Scientific term
- Key structural component
- Important relation target (even if only mentioned as "related to" something else)

List all entities in the `entities` array. Do not add new entities later — all must be declared upfront.

**Critical:** Once an entity is declared (e.g., "photosynthesis"), every later reference to it in `raw_entity_names` and `target_entity_name` must use the *exact same string form*. Do not pluralize, abbreviate, or rephrase. For example, if you declare "photosynthesis", do not later write "photosyntheses" or "the photosynthetic process" — stick to the exact form "photosynthesis".

### Step 2: Extract Knowledge Points
For each fact, claim, or relationship you identify, create a knowledge point. Each point should:
- Capture a single, testable claim (the `point` text)
- Specify what type of knowledge it represents (`knowledge_type`)
- List all entities this point is about (`raw_entity_names`) — all must already be in the `entities` list
- Identify relationships to other entities (`raw_relations`)

### Step 3: Specify Knowledge Type
Choose one of these types for each point:

1. **Definition** — Describes what something is, its fundamental nature, or essential characteristics.
   - Example: "Photosynthesis is the process by which plants convert light energy into chemical energy."
   - Use this for: taxonomies, class definitions, "is-a" relationships, essential properties

2. **Fact** — A specific, verifiable claim or observable property about one or more entities.
   - Example: "Chloroplasts contain thylakoids arranged in stacks called grana."
   - Use this for: measurements, empirical observations, structural details, quantities

3. **Procedural** — Describes a sequential process, method, or set of steps that a student or practitioner would follow.
   - Example: "To calculate molarity, divide moles of solute by liters of solution."
   - Use this for: step-by-step instructions, calculation methods, experimental procedures, actionable workflows

4. **Conceptual** — Explains relationships, theories, frameworks, or abstract connections between ideas.
   - Example: "The two stages of photosynthesis are coupled: light reactions produce ATP and NADPH that power the Calvin cycle."
   - Use this for: big-picture relationships, theoretical connections, interdependencies

### Step 4: Identify Relations
For each knowledge point, list **only the meaningful semantic connections** to other entities — not every entity that merely appears in the point. A relation represents one specific, testable claim about how two entities are connected. Do not emit a relation just because two entities co-occur in the same sentence; identify only the one or two most important connections that are worth testing in a question.

Choose the relation type that best describes the connection:

1. **related_to** — General association or relevance without a more specific relationship.
   - Example: mitochondria is related_to cellular respiration

2. **contrasts** — Shows a difference, opposition, or contrast.
   - Example: aerobic respiration contrasts anaerobic respiration

3. **prerequisite** — One thing is a prerequisite or necessary condition for another.
   - Example: water is a prerequisite for photosynthesis

4. **consequence** — One thing causes or leads to another as a result.
   - Example: deforestation is a consequence of slash-and-burn agriculture (or: deforestation is a consequence of [human activity])

5. **example** — One entity is a specific example or instance of another.
   - Example: glucose is an example of a simple sugar

6. **counter_example** — One entity contradicts or provides a counterexample to another.
   - Example: C4 photosynthesis is a counter_example to the standard Calvin cycle model (more nuanced)

## Verification Checklist (Before Finalizing)

Before outputting your final JSON, perform this self-check:
1. Scan all `raw_entity_names` across every knowledge point. For each name, verify it appears in the `entities` array (case-insensitive and exact string form).
2. Scan all `target_entity_name` values in `raw_relations`. Verify each one also appears in `entities`.
3. If you find any name that appears in a knowledge point but not in `entities`, **add it to the `entities` array now**, then proceed.

This check prevents validation errors and ensures your output is immediately usable.

## Output Requirements

- Output only valid JSON; no commentary or explanation outside the JSON block.
- Ensure all `raw_entity_names` and `target_entity_name` values exist in the `entities` array (case-insensitive match).
- Each knowledge point must have at least one entity in `raw_entity_names`.
- `raw_relations` may be empty if there are no notable relationships to other entities.
- `source_quote` is optional; include it if a direct quote from the text supports the relationship.

Now, extract the knowledge graph from the chunk above. Output only the JSON object, starting with `{{` and ending with `}}`.
"#,
        chunk_markdown, chunk_index_context
    )
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prompt_states_entity_consistency_rule() {
        let prompt = format_stage_a_graph_user_prompt("Sample chunk", "Context about other chunks");

        // Verify the prompt explicitly states the entity consistency rule
        assert!(
            prompt.contains("must also appear in the top-level `entities` array")
                || prompt.contains("must also appear in the `entities`"),
            "Prompt must state that entities must be declared in the entities list"
        );
    }

    #[test]
    fn test_prompt_contains_closed_guest_list_concept() {
        let prompt = format_stage_a_graph_user_prompt("Sample chunk", "Context");

        // Verify the concept of a "guest list" or similar is explained
        assert!(
            prompt.contains("guest list")
                || prompt.contains("closed list")
                || prompt.contains("declare"),
            "Prompt should explain the closed guest list concept"
        );
    }

    #[test]
    fn test_prompt_warns_about_undeclared_entities_mistake() {
        let prompt = format_stage_a_graph_user_prompt("Sample chunk", "Context");

        // Verify the specific failure mode is warned about
        assert!(
            prompt.contains("Common Mistake")
                || prompt.contains("common mistake")
                || prompt.contains("WRONG") && prompt.contains("thylakoid"),
            "Prompt should explicitly warn about undeclared entity mistake"
        );
    }

    #[test]
    fn test_prompt_includes_all_four_knowledge_types() {
        let prompt = format_stage_a_graph_user_prompt("Sample chunk", "Context");

        // Verify all four knowledge types are described
        assert!(
            prompt.contains("Definition"),
            "Prompt should describe Definition type"
        );
        assert!(prompt.contains("Fact"), "Prompt should describe Fact type");
        assert!(
            prompt.contains("Procedural"),
            "Prompt should describe Procedural type"
        );
        assert!(
            prompt.contains("Conceptual"),
            "Prompt should describe Conceptual type"
        );
    }

    #[test]
    fn test_prompt_includes_all_six_relation_types() {
        let prompt = format_stage_a_graph_user_prompt("Sample chunk", "Context");

        // Verify all six relation types are described
        assert!(
            prompt.contains("related_to"),
            "Prompt should describe related_to"
        );
        assert!(
            prompt.contains("contrasts"),
            "Prompt should describe contrasts"
        );
        assert!(
            prompt.contains("prerequisite"),
            "Prompt should describe prerequisite"
        );
        assert!(
            prompt.contains("consequence"),
            "Prompt should describe consequence"
        );
        assert!(
            prompt.contains("example"),
            "Prompt should describe example type"
        );
        assert!(
            prompt.contains("counter_example"),
            "Prompt should describe counter_example"
        );
    }

    #[test]
    fn test_prompt_includes_example_json_shape() {
        let prompt = format_stage_a_graph_user_prompt("Sample chunk", "Context");

        // Verify the JSON structure is shown in the prompt
        assert!(
            prompt.contains("\"entities\"") && prompt.contains("\"knowledge_points\""),
            "Prompt should include example JSON structure"
        );
    }

    #[test]
    fn test_prompt_references_chunk_markdown() {
        let chunk = "This is a test chunk about photosynthesis";
        let prompt = format_stage_a_graph_user_prompt(chunk, "Other chunks");

        // Verify the chunk content is included in the prompt
        assert!(
            prompt.contains(chunk),
            "Prompt should include the chunk markdown content"
        );
    }

    #[test]
    fn test_prompt_references_chunk_index_context() {
        let context = "Information about related chunks";
        let prompt = format_stage_a_graph_user_prompt("Chunk", context);

        // Verify the chunk context is included in the prompt
        assert!(
            prompt.contains(context),
            "Prompt should include the chunk index context"
        );
    }

    #[test]
    fn test_prompt_front_loads_entity_enumeration() {
        let prompt = format_stage_a_graph_user_prompt("Sample chunk", "Context");

        // Verify entity enumeration is presented as Step 1 before point extraction
        let step1_pos = prompt.find("Step 1");
        let step2_pos = prompt.find("Step 2");
        let entities_mention = prompt.find("Enumerate All Entities");

        assert!(
            step1_pos.is_some() && step2_pos.is_some(),
            "Prompt should have numbered steps"
        );
        assert!(
            entities_mention.is_some(),
            "Prompt should explicitly front-load entity enumeration"
        );
        assert!(
            step1_pos.unwrap() < step2_pos.unwrap(),
            "Entity enumeration should come before knowledge point extraction"
        );
    }
}
