# Rust Docs MCP Server

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

⭐ **Like this project? Please
[star the repository](https://github.com/Govcraft/rust-docs-mcp-server) on
GitHub to show your support and stay updated!** ⭐

## Motivation

Modern AI-powered coding assistants (like Cursor, Cline, Roo Code, etc.) excel
at understanding code structure and syntax but often struggle with the specifics
of rapidly evolving libraries and frameworks, especially in ecosystems like Rust
where crates are updated frequently. Their training data cutoff means they may
lack knowledge of the latest APIs, leading to incorrect or outdated code
suggestions.

This MCP server addresses this challenge by providing a focused, up-to-date
knowledge source for Rust crates. It intelligently discovers and lazily loads
documentation from your local Rust project, giving your LLM coding assistant a
tool (`query_rust_docs`) it can use _before_ writing code related to any crate
in your project.

When instructed to use this tool, the LLM can ask specific questions about a
crate's API or usage and receive answers derived directly from the _current_
documentation. This significantly improves the accuracy and relevance of the
generated code, reducing the need for manual correction and speeding up
development.

## Features

- **Automatic Crate Discovery:** Intelligently finds and provides documentation for
  all crates in your local Rust project's generated documentation.
- **Lazy Loading:** Only loads documentation and generates embeddings for a crate
  when it's first queried, saving time and resources.
- **On-Demand Documentation Generation:** Can automatically generate documentation
  even if the main project code doesn't compile (using `--generate-docs` flag).
- **Multiple Crate Support:** Run a single server instance to provide documentation for
  multiple Rust crates simultaneously.
- **Feature Support:** Allows specifying required crate features for
  documentation generation.
- **Semantic Search:** Uses OpenAI's `text-embedding-3-small` model to find the
  most relevant documentation sections for a given question.
- **LLM Summarization:** Leverages OpenAI's `gpt-4o-mini-2024-07-18` model to
  generate concise answers based _only_ on the retrieved documentation context.
- **Advanced Caching:** Uses a two-tier caching system:
  - **Traditional Cache:** Stores crate-specific embeddings in the user's data directory  
  - **Global Content Hash Cache:** Cross-project embedding reuse based solely on content hash, 
    avoiding redundant OpenAI API calls for identical content regardless of project
- **MCP Integration:** Runs as a standard MCP server over stdio, exposing tools
  and resources.

## Prerequisites

- **OpenAI API Key:** Needed for generating embeddings and summarizing answers.
  The server expects this key to be available in the `OPENAI_API_KEY`
  environment variable. (The server also requires network access to download
  crate dependencies and interact with the OpenAI API).

## Installation

The recommended way to install is to download the pre-compiled binary for your
operating system from the
[GitHub Releases page](https://github.com/Govcraft/rust-docs-mcp-server/releases).

1. Go to the
   [Releases page](https://github.com/Govcraft/rust-docs-mcp-server/releases).
2. Download the appropriate archive (`.zip` for Windows, `.tar.gz` for
   Linux/macOS) for your system.
3. Extract the `rustdocs_mcp_server` (or `rustdocs_mcp_server.exe`) binary.
4. Place the binary in a directory included in your system's `PATH` environment
   variable (e.g., `/usr/local/bin`, `~/bin`).

### Building from Source (Alternative)

If you prefer to build from source, you will need the
[Rust Toolchain](https://rustup.rs/) installed.

1. **Clone the repository:**
   ```bash
   git clone https://github.com/Govcraft/rust-docs-mcp-server.git
   cd rust-docs-mcp-server
   ```
2. **Build the server:**
   ```bash
   cargo build --release
   ```

## Usage

**Important Note for New Crates:**

When using the server with a crate for the first time (or with a new version/feature set), it needs to download the documentation and generate embeddings. This process can take some time, especially for crates with extensive documentation, and requires an active internet connection and OpenAI API key.

It is recommended to run the server once directly from your command line for any new crate configuration *before* adding it to your AI coding assistant (like Roo Code, Cursor, etc.). This allows the initial embedding generation and caching to complete. Once you see the server startup messages indicating it's ready (e.g., "MCP Server listening on stdio"), you can shut it down (Ctrl+C). Subsequent launches, including those initiated by your coding assistant, will use the cached data and start much faster.


### Running the Server

#### Basic Usage - Automatic Mode

The simplest way to use the server is to run it in your Rust project directory with no arguments:

```bash
# Set the API key (replace with your actual key)
export OPENAI_API_KEY="sk-..."

# Run server in automatic discovery mode with lazy loading (in your project directory)
rustdocs_mcp_server
```

In this mode, the server will:

1. Look for documentation in the `target/doc` directory
2. Discover all available crates
3. Lazily load documentation for each crate as it's first queried

#### Advanced Options

You can also run the server with various options:

```bash
# Generate documentation if it doesn't exist, even if the project doesn't compile
rustdocs_mcp_server --generate-docs

# Preload all available crates at startup (disable lazy loading)
rustdocs_mcp_server --preload

# Preload specific crates at startup, others will be lazily loaded
rustdocs_mcp_server tokio,serde,reqwest

# Preload only specific crates and disable lazy loading for others
rustdocs_mcp_server --preload tokio,serde,reqwest

# Run with specific features enabled for all crates
rustdocs_mcp_server -F full,compat

# Combine options
rustdocs_mcp_server tokio,hyper -F full,http2 --generate-docs

# Specify a different workspace path
rustdocs_mcp_server -w /path/to/your/project
```

The behavior of the `--preload` flag and crate name arguments works as follows:

1. **Default (no arguments)**: Lazy loading enabled, crates loaded on first query
2. **With crate names only**: Named crates preloaded at startup, other crates lazy loaded
3. **With --preload only**: All available crates preloaded, lazy loading disabled
4. **With both --preload and crate names**: Only named crates preloaded, lazy loading disabled

On the first run for a specific crate version _and feature set_, the server
will:

1. Download the crate documentation using `cargo doc` (with specified features).
2. Parse the HTML documentation.
3. Generate embeddings for the documentation content using the OpenAI API (this
   may take some time and incur costs, though typically only fractions of a US
   penny for most crates; even a large crate like `async-stripe` with over 5000
   documentation pages cost only $0.18 USD for embedding generation during
   testing).
4. Cache the documentation content and embeddings so that the cost isn't
   incurred again.
5. Start the MCP server.

Subsequent runs for the same crate version _and feature set_ will load the data
from the cache, making startup much faster.

### MCP Interaction

The server communicates using the Model Context Protocol over standard
input/output (stdio). It exposes the following:

- **Tool: `query_rust_docs`**
  - **Description:** Query documentation for any available Rust crate using
    semantic search and LLM summarization. The first time you query a crate,
    it will be automatically loaded and cached.
  - **Input Schema:**
    ```json
    {
      "type": "object",
      "properties": {
        "question": {
          "type": "string",
          "description": "The specific question about the crate's API or usage."
        },
        "crate_name": {
          "type": "string",
          "description": "Name of the Rust crate to query documentation for."
        }
      },
      "required": ["question", "crate_name"]
    }
    ```
  - **Output:** A text response containing the answer generated by the LLM based
    on the relevant documentation context, prefixed with
    `From <crate_name> docs:`.
  - **Example MCP Call:**
    ```json
    {
      "jsonrpc": "2.0",
      "method": "callTool",
      "params": {
        "tool_name": "query_rust_docs",
        "arguments": {
          "question": "How do I make a simple GET request with reqwest?",
          "crate_name": "reqwest"
        }
      },
      "id": 1
    }
    ```
  - **Error Handling:** If the requested crate isn't found, the server will suggest similar crates that are available.

- **Resource: `crate://<crate_name>`**
  - **Description:** Provides the name of each Rust crate this server instance is
    configured for.
  - **URI:** `crate://<crate_name>` (e.g., `crate://serde`, `crate://reqwest`)
  - **Content:** Plain text containing the crate name.

- **Logging:** The server sends informational logs (startup messages, query
  processing steps) back to the MCP client via `logging/message` notifications.

### Example Client Configuration (Roo Code)

You can configure MCP clients like Roo Code to run this server for multiple crates. Here's an example snippet for Roo
Code's `mcp_settings.json` file:

```json
{
  "mcpServers": {
    "rust-docs": {
      "command": "/path/to/your/rustdocs_mcp_server",
      "args": [
        "reqwest@0.12,tokio,serde_json"
      ],
      "env": {
        "OPENAI_API_KEY": "YOUR_OPENAI_API_KEY_HERE"
      },
      "disabled": false,
      "alwaysAllow": []
    },
    "rust-docs-async-stripe": {
      "command": "rustdocs_mcp_server",
      "args": [
        "async-stripe@0.40",
        "-F",
        "runtime-tokio-hyper-rustls"
      ],
      "env": {
        "OPENAI_API_KEY": "YOUR_OPENAI_API_KEY_HERE"
      },
      "disabled": false,
      "alwaysAllow": []
    }
  }
}
```

**Note:**

- Replace `/path/to/your/rustdocs_mcp_server` with the actual path to the
  compiled binary on your system if it isn't in your PATH.
- Replace `YOUR_OPENAI_API_KEY_HERE` with your actual OpenAI API key.
- The keys (`rust-docs`, `rust-docs-async-stripe`) are arbitrary names
  you choose to identify the server instances within Roo Code.

### Example Client Configuration (Claude Desktop)

For Claude Desktop users, you can configure the server in the MCP settings.
Here's an example configuration:

```json
{
  "mcpServers": {
    "rust-docs": {
      "command": "/path/to/your/rustdocs_mcp_server",
      "args": [
        "serde@^1.0,tokio,reqwest"
      ]
    },
    "rust-docs-async-stripe": {
      "command": "rustdocs_mcp_server",
      "args": [
        "async-stripe@0.40",
        "-F",
        "runtime-tokio-hyper-rustls"
      ]
    }
  }
}
```

**Note:**

- Ensure `rustdocs_mcp_server` is in your system's PATH or provide the full path
  (e.g., `/path/to/your/rustdocs_mcp_server`).
- The keys (`rust-docs`, `rust-docs-async-stripe`) are arbitrary names
  you choose to identify the server instances.
- Remember to set the `OPENAI_API_KEY` environment variable where Claude Desktop
  can access it (this might be system-wide or via how you launch Claude
  Desktop). Claude Desktop's MCP configuration might not directly support
  setting environment variables per-server like Roo Code.
- The example shows how to add the `-F` argument for crates like `async-stripe`
  that require specific features.

### Caching

The server uses a highly efficient two-tier caching system to minimize OpenAI API costs:

#### Global Content Hash Cache

- **Location:** `~/.local/share/rustdocs-mcp-server/embeddings-v2/` (Linux/macOS) or equivalent on Windows
- **Strategy:** Content-addressed storage where each file is named by the content hash
- **Benefits:**
  - Cross-project reuse - embeddings are valid across any projects
  - Immune to version/feature changes - only content matters
  - In-memory cache for frequently used embeddings
  - Fast FNV-1a hashing algorithm for efficiency
  
#### Traditional Per-Crate Cache (Legacy Support)

- **Location:** `~/.local/share/rustdocs-mcp-server/<crate_name>/<features_hash>/embeddings.bin`
- **Purpose:** Provides compatibility with existing installations

#### Smart Regeneration

The server is intelligent about embedding generation:
- Checks global cache first
- Falls back to traditional cache  
- Only generates embeddings for content not found in either cache
- Automatically stores new embeddings in both caches

## How it Works

1. **Initialization:** Parses the crate specifications and optional features from
   the command line using `clap`.
2. **Cache Check:** Looks for a pre-existing cache file for each specific crate,
   version requirement, and feature set.
3. **Documentation Generation (if cache miss):**
   - Creates a temporary Rust project depending only on the target crate,
     enabling the specified features in its `Cargo.toml`.
   - Runs `cargo doc` using the `cargo` library API to generate HTML
     documentation in the temporary directory.
   - Dynamically locates the correct output directory within `target/doc` by
     searching for the subdirectory containing `index.html`.
4. **Content Extraction (if cache miss):**
   - Walks the generated HTML files within the located documentation directory.
   - Uses the `scraper` crate to parse each HTML file and extract text content
     from the main content area (`<section id="main-content">`).
5. **Embedding Generation (if cache miss):**
   - Uses the `async-openai` crate and `tiktoken-rs` to generate embeddings for
     each extracted document chunk using the `text-embedding-3-small` model.
   - Calculates the estimated cost based on the number of tokens processed.
6. **Caching (if cache miss):** Saves the extracted document content and their
   corresponding embeddings to the cache file (path includes features hash)
   using `bincode`.
7. **Server Startup:** Initializes the `RustDocsServer` with automatic crate discovery
   and lazy loading capabilities.
8. **MCP Serving:** Starts the MCP server using `rmcp` over stdio.
9. **Query Handling (`query_rust_docs` tool):**
   - Checks if the requested crate is already loaded into the server.
   - If not loaded and lazy loading is enabled, attempts to load it automatically.
   - If the crate isn't found, suggests similar crates that are available.
   - Generates an embedding for the user's question.
   - Calculates the cosine similarity between the question embedding and all
     cached document embeddings for the specified crate.
   - Identifies the document chunk with the highest similarity.
   - Sends the user's question and the content of the best-matching document
     chunk to the `gpt-4o-mini-2024-07-18` model via the OpenAI API.
   - The LLM is prompted to answer the question based _only_ on the provided
     context.
   - Returns the LLM's response to the MCP client.

## License

This project is licensed under the MIT License.

Copyright (c) 2025 Govcraft

## Sponsor

If you find this project helpful, consider sponsoring the development!

[![Sponsor on GitHub](https://img.shields.io/badge/Sponsor-%E2%9D%A4-%23db61a2?logo=GitHub)](https://github.com/sponsors/Govcraft)