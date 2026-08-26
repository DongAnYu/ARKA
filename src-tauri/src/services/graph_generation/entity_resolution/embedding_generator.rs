//! Entity embedding orchestration for semantic resolution.
//!
//! This module connects deterministic graph contexts to the provider-neutral
//! embedding service. It preserves graph entity order so each validated vector
//! is associated with the stable entity ID that produced its input text.

use crate::services::embedding::{EmbeddingService, EmbeddingServiceError, EmbeddingVector};
use crate::services::graph_generation::types::PropositionGraph;

use super::context_builder::build_entity_contexts;

/// One validated embedding associated with its stable graph entity ID.
#[derive(Debug, Clone, PartialEq)]
pub struct EntityEmbedding {
    pub entity_id: String,
    pub vector: EmbeddingVector,
}

/// Builds entity contexts, embeds them as one ordered batch, and associates
/// every returned vector with the stable ID of its source entity.
///
/// `EmbeddingService` validates vector count, dimensions, finite values, and
/// provider response ordering before this function creates any associations.
/// An empty graph returns an empty result without contacting the provider.
pub async fn generate_entity_embeddings(
    graph: &PropositionGraph,
    embedding_service: &EmbeddingService,
    max_points_per_entity: usize,
) -> Result<Vec<EntityEmbedding>, EmbeddingServiceError> {
    let contexts = build_entity_contexts(graph, max_points_per_entity);
    let inputs = contexts
        .iter()
        .map(|context| context.embedding_text())
        .collect::<Vec<_>>();
    let vectors = embedding_service.embed_batch(&inputs).await?.into_vectors();

    Ok(contexts
        .into_iter()
        .zip(vectors)
        .map(|(context, vector)| EntityEmbedding {
            entity_id: context.entity_id,
            vector,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::embedding::{EmbeddingConfig, EmbeddingProvider};
    use crate::services::graph_generation::types::{
        EntityNode, KnowledgePoint, KnowledgeType, PropositionGraph,
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::sync::oneshot;

    fn entity(id: &str, name: &str, aliases: &[&str]) -> EntityNode {
        EntityNode {
            id: id.to_string(),
            canonical_name: name.to_string(),
            aliases: aliases.iter().map(|alias| alias.to_string()).collect(),
            chunk_ids: Vec::new(),
        }
    }

    fn point(id: &str, text: &str, entity_ids: &[&str]) -> KnowledgePoint {
        KnowledgePoint {
            id: id.to_string(),
            point: text.to_string(),
            knowledge_type: KnowledgeType::Fact,
            chunk_id: String::from("chunk-1"),
            raw_entity_names: Vec::new(),
            raw_relations: Vec::new(),
            entity_ids: entity_ids.iter().map(|id| id.to_string()).collect(),
        }
    }

    fn graph() -> PropositionGraph {
        PropositionGraph {
            entities: vec![
                entity("entity-co2", "CO₂", &["CO₂", "CO2"]),
                entity("entity-oxygen", "oxygen", &["oxygen", "O₂"]),
            ],
            knowledge_points: vec![
                point(
                    "kp-1",
                    "CO₂ is attached to RuBP by RuBisCO.",
                    &["entity-co2"],
                ),
                point(
                    "kp-2",
                    "Oxygen is released when water is split.",
                    &["entity-oxygen"],
                ),
            ],
            relations: Vec::new(),
        }
    }

    fn service(base_url: &str) -> EmbeddingService {
        let config = EmbeddingConfig::new(
            EmbeddingProvider::Ollama,
            base_url,
            "nomic-embed-text",
            5,
            None,
        )
        .expect("test embedding configuration should be valid");
        EmbeddingService::new(config).expect("test embedding service should build")
    }

    async fn mock_ollama(
        status_line: &str,
        response_body: &str,
    ) -> (String, oneshot::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock listener should bind");
        let address = listener
            .local_addr()
            .expect("mock listener should have an address");
        let status_line = status_line.to_string();
        let response_body = response_body.to_string();
        let (request_sender, request_receiver) = oneshot::channel();

        tokio::spawn(async move {
            let (mut socket, _) = listener
                .accept()
                .await
                .expect("mock listener should accept a request");
            let mut request = Vec::new();
            let mut chunk = [0_u8; 1024];

            loop {
                let bytes_read = socket
                    .read(&mut chunk)
                    .await
                    .expect("mock server should read the request");
                if bytes_read == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..bytes_read]);

                let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n")
                else {
                    continue;
                };
                let headers = String::from_utf8_lossy(&request[..header_end]);
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    })
                    .unwrap_or(0);
                if request.len() >= header_end + 4 + content_length {
                    break;
                }
            }

            let _ = request_sender.send(String::from_utf8_lossy(&request).into_owned());
            let response = format!(
                "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
                response_body.len()
            );
            socket
                .write_all(response.as_bytes())
                .await
                .expect("mock server should write the response");
        });

        (format!("http://{address}"), request_receiver)
    }

    #[tokio::test]
    async fn embeds_contexts_and_preserves_entity_order() {
        let (base_url, request_receiver) =
            mock_ollama("200 OK", r#"{"embeddings":[[0.1,0.2],[0.3,0.4]]}"#).await;

        let embeddings = generate_entity_embeddings(&graph(), &service(&base_url), 2)
            .await
            .expect("entity contexts should embed");

        assert_eq!(embeddings.len(), 2);
        assert_eq!(embeddings[0].entity_id, "entity-co2");
        assert_eq!(embeddings[0].vector.values(), &[0.1, 0.2]);
        assert_eq!(embeddings[1].entity_id, "entity-oxygen");
        assert_eq!(embeddings[1].vector.values(), &[0.3, 0.4]);

        let request = request_receiver
            .await
            .expect("mock server should capture the request");
        let (_, body) = request
            .split_once("\r\n\r\n")
            .expect("HTTP request should contain a body");
        let payload: serde_json::Value =
            serde_json::from_str(body).expect("request body should be JSON");

        assert_eq!(
            payload["input"][0],
            "Entity: CO₂\nAliases: CO₂, CO2\nContext:\n- CO₂ is attached to RuBP by RuBisCO."
        );
        assert_eq!(payload["input"][1], "Entity: oxygen\nAliases: oxygen, O₂\nContext:\n- Oxygen is released when water is split.");
    }

    #[tokio::test]
    async fn empty_graph_returns_without_contacting_the_provider() {
        let graph = PropositionGraph {
            entities: Vec::new(),
            knowledge_points: Vec::new(),
            relations: Vec::new(),
        };

        let embeddings = generate_entity_embeddings(&graph, &service("http://127.0.0.1:1"), 3)
            .await
            .expect("empty graph should not require a provider request");

        assert!(embeddings.is_empty());
    }

    #[tokio::test]
    async fn provider_errors_are_returned_without_entity_associations() {
        let (base_url, _request_receiver) = mock_ollama(
            "500 Internal Server Error",
            r#"{"error":"model unavailable"}"#,
        )
        .await;

        let error = generate_entity_embeddings(&graph(), &service(&base_url), 2)
            .await
            .expect_err("provider failure should stop entity association");

        assert!(matches!(
            error,
            EmbeddingServiceError::HttpStatus {
                status: reqwest::StatusCode::INTERNAL_SERVER_ERROR,
                message
            } if message == "model unavailable"
        ));
    }
}
