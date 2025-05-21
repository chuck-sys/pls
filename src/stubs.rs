use tree_sitter::{Node, Parser, Query, QueryCursor, StreamingIterator};

use tree_sitter_php::language_php;

use std::collections::{HashMap, HashSet};
use std::fmt::Display;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, LazyLock};

static CONST_QUERY: LazyLock<Query> =
    LazyLock::new(|| Query::new(&language_php(), "(array_creation_expression) @a").unwrap());

pub struct FileMapping {
    mapping: HashMap<String, Arc<PathBuf>>,

    /// Set of files involved, interned to probably keep memory usage low.
    files: HashSet<Arc<PathBuf>>,
}

#[derive(Debug)]
pub enum MappingError {
    IOError(std::io::Error),
    NoMappingFound,
    NoChildFound,
    UnexpectedType(&'static str, &'static str),
    MissingNameNode,
    BadStubName(String),
}

impl From<std::io::Error> for MappingError {
    fn from(value: std::io::Error) -> Self {
        Self::IOError(value)
    }
}

impl Display for MappingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MappingError::IOError(error) => error.fmt(f),
            MappingError::NoMappingFound => write!(f, "no mapping found"),
            MappingError::NoChildFound => write!(f, "no child found"),
            MappingError::MissingNameNode => write!(f, "missing name node"),
            MappingError::UnexpectedType(actual, expected) => {
                write!(f, "found type {} (expected {})", actual, expected)
            }
            MappingError::BadStubName(name) => {
                write!(f, "unknown stubs mapping with name {name}")
            }
        }
    }
}

impl std::error::Error for MappingError {}

impl FileMapping {
    fn node_to_string(node: Node<'_>, content: &str) -> Result<String, MappingError> {
        if node.kind() == "string" {
            let mut range = node.byte_range();
            range.start += 1;
            range.end -= 1;
            Ok(content[range].replace("\\\\", "\\"))
        } else {
            Err(MappingError::UnexpectedType(node.kind(), "string"))
        }
    }

    fn node_to_single_mapping(
        node: Node<'_>,
        content: &str,
    ) -> Result<(String, String), MappingError> {
        let item1 =
            Self::node_to_string(node.child(0).ok_or(MappingError::NoChildFound)?, content)?;
        let item2 =
            Self::node_to_string(node.child(2).ok_or(MappingError::NoChildFound)?, content)?;
        Ok((item1, item2))
    }

    fn node_to_mapping(node: Node<'_>, content: &str) -> Result<Self, MappingError> {
        let mut cursor = QueryCursor::new();
        let mut captures = cursor.captures(&CONST_QUERY, node, content.as_bytes());
        let mut files: HashSet<Arc<PathBuf>> = HashSet::new();
        let mut mapping = HashMap::new();

        while let Some((m, _)) = captures.next() {
            for c in m.captures.iter() {
                let array_root = c.node;

                let mut cursor = array_root.walk();
                for child in array_root.children(&mut cursor) {
                    if child.kind() != "array_element_initializer" {
                        continue;
                    }

                    let (item0, item1) = Self::node_to_single_mapping(child, content)?;
                    let file = PathBuf::from_str(&item1).unwrap();

                    let file = if files.contains(&file) {
                        files.get(&file).unwrap().clone()
                    } else {
                        Arc::from(file)
                    };

                    mapping.insert(item0, file.clone());
                    files.insert(file);
                }
            }
        }

        Ok(Self { mapping, files })
    }

    pub fn from_filename<P>(filename: P, parser: &mut Parser) -> Result<Self, MappingError>
    where
        P: AsRef<Path>,
    {
        let f = File::open(filename)?;
        let mut buf = BufReader::new(f);
        let mut contents = String::new();
        let _ = buf.read_to_string(&mut contents)?;

        let tree = parser.parse(contents.as_str(), None).unwrap();
        let root_node = tree.root_node();

        Self::node_to_mapping(root_node, &contents)
    }
}

#[cfg(test)]
mod test {
    use tree_sitter::Parser;
    use tree_sitter_php::language_php;

    fn parser() -> Parser {
        let mut parser = Parser::new();
        parser
            .set_language(&language_php())
            .expect("error loading PHP grammar");

        parser
    }

    const SOURCE: &'static str = "<?php
final class PhpStormStubsMap
{
const DIR = __DIR__;

const CLASSES = [
  'AMQPBasicProperties' => 'amqp/amqp.php',
  'AMQP\\annel' => 'amqp/amqp.php',
  'AMQPChannelException' => 'amqp/amqp.php',
  'AMQPConnection' => 'amqp/amqp.php',
  'AMQPConnectionException' => 'amqp/amqp.php',
  'AMQPDecimal' => 'amqp/amqp.php',
  'AMQPEnvelope' => 'amqp/amqp.php',
  'AMQP\\Envelope\\Exception' => 'amqp/amqp.php',
  ];
}";

    use super::FileMapping;
    use std::path::PathBuf;
    use std::str::FromStr;

    #[test]
    fn snippet_of_phpstorm() {
        let tree = parser().parse(SOURCE, None).unwrap();
        let root = tree.root_node();
        let file_mapping = FileMapping::node_to_mapping(root, SOURCE).unwrap();

        assert_eq!(file_mapping.files.len(), 1);
        assert_eq!(file_mapping.mapping.len(), 8);
        assert!(file_mapping
            .files
            .contains(&PathBuf::from_str("amqp/amqp.php").unwrap()));
        assert!(file_mapping.mapping.contains_key("AMQP\\annel"));
        assert!(file_mapping
            .mapping
            .contains_key("AMQP\\Envelope\\Exception"));
    }

    #[test]
    fn parse_phpstorm_stubs() {
        let file_name = PathBuf::from_str("phpstorm-stubs/PhpStormStubsMap.php").unwrap();
        let mut p = parser();
        let file_mapping = FileMapping::from_filename(&file_name, &mut p).unwrap();
        assert!(file_mapping.files.len() <= file_mapping.mapping.len());
        assert_eq!(
            file_mapping
                .mapping
                .get("array_filter")
                .unwrap()
                .to_path_buf(),
            PathBuf::from_str("standard/standard_9.php").unwrap()
        );
    }
}
