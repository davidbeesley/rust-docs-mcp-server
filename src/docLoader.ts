import { HTMLReader } from "@llamaindex/readers/html";
import { glob, GlobOptions } from "glob";
import path from "path";
import fs from "fs/promises";
import { Document } from "llamaindex";

// Define Metadata type (adjust if you know the exact shape)
interface Metadata {
  [key: string]: any; // Generic for now; refine if metadata has specific fields
}

export async function loadCrateDocs(
  rootDocsPath: string,
  crateName: string
): Promise<Document<Metadata>[]> {
  const docsPath = path.join(rootDocsPath, crateName);
  const pattern: string = "**/*.html";
  const options: GlobOptions = {
    cwd: docsPath,
    ignore: "**/index.html",
    withFileTypes: false,
  };

  const htmlFiles: string[] = (await glob(pattern, options)) as string[];
  console.log("found # html files (initial):", htmlFiles.length);

  // Check the dangerous flag
  const includeAllDangerously = process.env.INCLUDE_ALL_DOCS_DANGEROUSLY === "true";
  let filesToProcess: string[] = [];

  if (includeAllDangerously) {
    console.warn(
      `WARNING: INCLUDE_ALL_DOCS_DANGEROUSLY is enabled for ${crateName}. Processing all ${htmlFiles.length} HTML files. This may incur significant time or cost (e.g., embedding calls).`
    );
    filesToProcess = htmlFiles;
  } else {
    // Group files by basename and count occurrences
    const basenameGroups: { [key: string]: string[] } = {};
    htmlFiles.forEach((file: string) => {
      const basename: string = path.basename(file);
      if (!basenameGroups[basename]) {
        basenameGroups[basename] = [];
      }
      basenameGroups[basename].push(file);
    });

    // Filter for duplicate basenames and pick largest file
    for (const [basename, files] of Object.entries(basenameGroups)) {
      if (files.length > 1) {
        const largestFile = await files.reduce(
          async (largestPromise, current) => {
            const largest = await largestPromise;
            const largestStats = await fs.stat(path.join(docsPath, largest));
            const currentStats = await fs.stat(path.join(docsPath, current));
            return largestStats.size > currentStats.size ? largest : current;
          },
          Promise.resolve(files[0])
        );
        filesToProcess.push(largestFile);
      }
    }
    console.log(
      "found # unique duplicate html files for crate " + crateName + ":",
      filesToProcess.length
    );
  }

  // Process the selected files with HTMLReader
  const htmlReader: HTMLReader = new HTMLReader();
  const docs: Document<Metadata>[][] = await Promise.all(
    filesToProcess.map(async (filePath: string) => {
      const fullFilePath: string = path.join(docsPath, filePath);
      const fileDocs: Document<Metadata>[] = await htmlReader.loadData(fullFilePath);
      console.log("Loaded:", filePath);

      // Modify existing Document objects to add id_
      return fileDocs.map((doc: Document<Metadata>) => {
        doc.id_ = path.relative(docsPath, filePath);
        return doc;
      });
    })
  );

  const foundDocs: Document<Metadata>[] = docs.flat();
  console.log("found # docs from processed files:", foundDocs.length);
  return foundDocs;
}

export function combineDocuments(
  docs1: Document<Metadata>[],
  docs2: Document<Metadata>[]
): Document<Metadata>[] {
  return [...docs1, ...docs2];
}