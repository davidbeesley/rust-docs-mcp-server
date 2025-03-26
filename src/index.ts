import { Server } from "@modelcontextprotocol/sdk/server/index.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import {
  CallToolRequestSchema,
  ErrorCode,
  ListToolsRequestSchema,
  McpError,
  type CallToolRequest,
} from "@modelcontextprotocol/sdk/types.js";
import {
  VectorStoreIndex,
  storageContextFromDefaults,
  EngineResponse,
} from "llamaindex";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { loadCrateDocs } from "./docLoader.js";
import { env } from "node:process";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

class RustDocsServer {
  private server: Server;
  private index!: VectorStoreIndex;
  private persistDir: string;
  private docsPath: string;
  private crateName: string;
  private toolName: string;

  constructor(docsPath: string, crateName: string) {
    this.server = new Server(
      {
        name: "rust-docs-server",
        version: "1.0.0",
      },
      {
        capabilities: {
          tools: {},
        },
      }
    );

    this.crateName = crateName;
    this.persistDir = path.join(__dirname, "storage", this.crateName);
    this.docsPath = docsPath;
    this.setupToolHandlers();
    this.toolName = `query_rust_docs_${this.crateName}`;
  }

  private async initializeIndex() {
    try {
      const indexExists =
        fs.existsSync(path.join(this.persistDir, "vector_store.json")) &&
        fs.existsSync(path.join(this.persistDir, "doc_store.json")) &&
        fs.existsSync(path.join(this.persistDir, "index_store.json"));

      if (indexExists) {
        console.log("Loading existing vector store from", this.persistDir);
        const storageContext = await storageContextFromDefaults({
          persistDir: this.persistDir,
        });
        this.index = await VectorStoreIndex.init({ storageContext });
        return;
      } else {
        // Clear only the files inside the persist directory, not the directory itself
        if (fs.existsSync(this.persistDir)) {
          const files = fs.readdirSync(this.persistDir);
          for (const file of files) {
            fs.unlinkSync(path.join(this.persistDir, file));
          }
        } else {
          // Ensure the directory exists if it was never created
          fs.mkdirSync(this.persistDir, { recursive: true });
        }
      }

      const storageContext = await storageContextFromDefaults({
        persistDir: this.persistDir,
      });
      this.index = await VectorStoreIndex.fromDocuments(
        await loadCrateDocs(this.docsPath, this.crateName),
        {
          storageContext,
        }
      );

      console.log("Rust documentation indexed and persisted successfully");
    } catch (error) {
      console.error("Failed to initialize index:", error);
      throw new Error("Failed to initialize Rust documentation index");
    }
  }

  private setupToolHandlers() {
    this.server.setRequestHandler(ListToolsRequestSchema, async () => ({
      tools: [
        {
          name: this.toolName,
          description: `Query the official Rust documentation for the '${this.crateName}' crate. Use this tool to retrieve detailed information about '${this.crateName}'’s API, including structs, traits, enums, constants, and functions. Ideal for answering technical questions about how to use '${this.crateName}' in Rust projects, such as understanding specific methods, configuration options, or integration details. Additionally, leverage this tool to ensure accuracy of written code by verifying API usage and to resolve Clippy or lint errors by clarifying correct implementations. For example, use it for questions like "How do I configure routing in ${this.crateName}?", "What does this ${this.crateName} struct do?", "Is this ${this.crateName} method call correct?", or "How do I fix a Clippy warning about ${this.crateName} in my code?"`,
          inputSchema: {
            type: "object",
            properties: {
              question: {
                type: "string",
                description: `The specific question about the '${this.crateName}' crate’s API or usage. Should be a clear, focused query about its functionality, such as "What are the parameters of ${this.crateName}’s main struct?", "How do I use ${this.crateName} for async operations?", or "How do I resolve a Clippy error related to ${this.crateName}?"`,
              },
              crate: {
                type: "string",
                description: `The name of the crate to query. Must match the current crate, which is '${this.crateName}'. This ensures the question is routed to the correct documentation.`,
                enum: [this.crateName],
              },
            },
            required: ["question", "crate"],
          },
        },
      ],
    }));

    this.server.setRequestHandler(
      CallToolRequestSchema,
      async (request: CallToolRequest) => {
        if (request.params.name !== this.toolName) {
          throw new McpError(
            ErrorCode.MethodNotFound,
            `Unknown tool: ${request.params.name}`
          );
        }

        const { question, crate } = request.params.arguments as {
          question: string;
          crate: string;
        };

        if (crate !== this.crateName) {
          throw new McpError(
            ErrorCode.InvalidParams,
            `This server only supports queries for '${this.crateName}', not '${crate}'`
          );
        }

        try {
          const queryEngine = this.index.asQueryEngine();
          const response: EngineResponse = await queryEngine.query({
            query: question,
            stream: false,
          });

          return {
            content: [
              {
                type: "text",
                text: `From ${crate} docs: ${response.message.content}`,
              },
            ],
          };
        } catch (error) {
          console.error("Query failed:", error);
          throw new McpError(
            ErrorCode.InternalError,
            "Failed to query Rust documentation"
          );
        }
      }
    );
  }
  async run() {
    await this.initializeIndex();
    const transport = new StdioServerTransport();
    await this.server.connect(transport);
    console.error("Rust Docs MCP server running");
  }
}
// throw an error if the environment variables are not set
if (!process.env.CRATE_NAME) {
  throw new Error("CRATE_NAME environment variable must be set");
}
if (!process.env.DOCS_PATH) {
  throw new Error("DOCS_PATH environment variable must be set");
}
if (!process.env.OPENAI_API_KEY) {
  throw new Error("OPENAI_API_KEY environment variable must be set");
}

const server = new RustDocsServer(
  process.env.DOCS_PATH,
  process.env.CRATE_NAME
);
server.run().catch(console.error);
