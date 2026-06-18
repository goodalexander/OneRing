use std::io;
use std::path::Path;

use codex_extension_api::LoadUserInstructionsFuture;
use codex_extension_api::LoadedUserInstructions;
use codex_extension_api::UserInstructions;
use codex_extension_api::UserInstructionsProvider;
use codex_utils_absolute_path::AbsolutePathBuf;

const DEFAULT_AGENTS_MD_FILENAME: &str = "AGENTS.md";
const LOCAL_AGENTS_MD_FILENAME: &str = "AGENTS.override.md";
const INSTRUCTIONS_DIR_NAME: &str = "instructions";
const INSTRUCTIONS_FILE_EXTENSION: &str = "md";

#[derive(Clone, Debug)]
struct HomeInstructionFile {
    text: String,
    source: AbsolutePathBuf,
}

/// Loads user instructions from a Codex home directory.
#[derive(Clone, Debug)]
pub struct CodexHomeUserInstructionsProvider {
    codex_home: AbsolutePathBuf,
}

impl CodexHomeUserInstructionsProvider {
    /// Creates a provider rooted at the supplied absolute Codex home directory.
    pub fn new(codex_home: AbsolutePathBuf) -> Self {
        Self { codex_home }
    }

    async fn load_from_codex_home(&self) -> LoadedUserInstructions {
        let mut warnings = Vec::new();
        let mut files = Vec::new();

        if let Some(instructions) = self.load_agents_md(&mut warnings).await {
            files.push(instructions);
        }

        files.extend(self.load_instructions_dir(&mut warnings).await);

        let Some(instructions) = self.combine_instruction_files(files) else {
            return LoadedUserInstructions {
                instructions: None,
                warnings,
            };
        };

        LoadedUserInstructions {
            instructions: Some(UserInstructions {
                text: instructions.text,
                source: instructions.source,
            }),
            warnings,
        }
    }

    async fn load_agents_md(&self, warnings: &mut Vec<String>) -> Option<HomeInstructionFile> {
        for candidate in [LOCAL_AGENTS_MD_FILENAME, DEFAULT_AGENTS_MD_FILENAME] {
            let path = self.codex_home.join(candidate);
            match tokio::fs::metadata(path.as_path()).await {
                Ok(metadata) if !metadata.is_file() => continue,
                Ok(_) => {}
                Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
                Err(err) => {
                    warnings.push(format!(
                        "Failed to read global AGENTS.md instructions from `{}`: {err}",
                        path.display()
                    ));
                    continue;
                }
            }
            let data = match tokio::fs::read(path.as_path()).await {
                Ok(data) => data,
                Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
                Err(err) => {
                    warnings.push(format!(
                        "Failed to read global AGENTS.md instructions from `{}`: {err}",
                        path.display()
                    ));
                    continue;
                }
            };
            let contents = String::from_utf8_lossy(&data);
            let trimmed = contents.trim();
            if !trimmed.is_empty() {
                return Some(HomeInstructionFile {
                    text: trimmed.to_string(),
                    source: path,
                });
            }
        }

        None
    }

    async fn load_instructions_dir(&self, warnings: &mut Vec<String>) -> Vec<HomeInstructionFile> {
        let instructions_dir = self.codex_home.join(INSTRUCTIONS_DIR_NAME);
        match tokio::fs::metadata(instructions_dir.as_path()).await {
            Ok(metadata) if !metadata.is_dir() => return Vec::new(),
            Ok(_) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Vec::new(),
            Err(err) => {
                warnings.push(format!(
                    "Failed to read global instructions directory from `{}`: {err}",
                    instructions_dir.display()
                ));
                return Vec::new();
            }
        }

        let mut read_dir = match tokio::fs::read_dir(instructions_dir.as_path()).await {
            Ok(read_dir) => read_dir,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Vec::new(),
            Err(err) => {
                warnings.push(format!(
                    "Failed to read global instructions directory from `{}`: {err}",
                    instructions_dir.display()
                ));
                return Vec::new();
            }
        };

        let mut candidates = Vec::new();
        loop {
            match read_dir.next_entry().await {
                Ok(Some(entry)) => {
                    let path = entry.path();
                    if is_markdown_file(&path) {
                        candidates.push(path);
                    }
                }
                Ok(None) => break,
                Err(err) => {
                    warnings.push(format!(
                        "Failed to read global instructions directory entry from `{}`: {err}",
                        instructions_dir.display()
                    ));
                    break;
                }
            }
        }
        candidates.sort();

        let mut files = Vec::new();
        for path in candidates {
            let Ok(source) = AbsolutePathBuf::try_from(path.clone()) else {
                warnings.push(format!(
                    "Failed to read global instruction file from `{}`: path is not absolute",
                    path.display()
                ));
                continue;
            };

            match tokio::fs::metadata(&path).await {
                Ok(metadata) if !metadata.is_file() => continue,
                Ok(_) => {}
                Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
                Err(err) => {
                    warnings.push(format!(
                        "Failed to read global instruction file from `{}`: {err}",
                        path.display()
                    ));
                    continue;
                }
            }

            let data = match tokio::fs::read(&path).await {
                Ok(data) => data,
                Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
                Err(err) => {
                    warnings.push(format!(
                        "Failed to read global instruction file from `{}`: {err}",
                        path.display()
                    ));
                    continue;
                }
            };
            let contents = String::from_utf8_lossy(&data);
            let trimmed = contents.trim();
            if !trimmed.is_empty() {
                files.push(HomeInstructionFile {
                    text: trimmed.to_string(),
                    source,
                });
            }
        }

        files
    }

    fn combine_instruction_files(
        &self,
        files: Vec<HomeInstructionFile>,
    ) -> Option<HomeInstructionFile> {
        let mut files = files.into_iter();
        let first = files.next()?;
        let mut combined = first.text;
        let mut source = first.source;
        let mut has_multiple_sources = false;

        for file in files {
            combined.push_str("\n\n");
            combined.push_str(&file.text);
            has_multiple_sources = true;
        }

        if has_multiple_sources {
            source = self.codex_home.join(INSTRUCTIONS_DIR_NAME);
        }

        Some(HomeInstructionFile {
            text: combined,
            source,
        })
    }
}

fn is_markdown_file(path: &Path) -> bool {
    path.extension().and_then(|extension| extension.to_str()) == Some(INSTRUCTIONS_FILE_EXTENSION)
}

impl UserInstructionsProvider for CodexHomeUserInstructionsProvider {
    fn load_user_instructions(&self) -> LoadUserInstructionsFuture<'_> {
        Box::pin(self.load_from_codex_home())
    }
}

#[cfg(test)]
mod tests;
