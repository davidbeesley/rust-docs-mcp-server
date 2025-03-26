import { HTMLReader } from "@llamaindex/readers/html";
import { glob } from "glob";
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
  const options: import("glob").GlobOptions = {
    cwd: docsPath,
    ignore: "**/index.html",
    withFileTypes: false,
  };

  const htmlFiles: string[] = await glob(pattern, options) as string[];
  console.log("found # html files (initial):", htmlFiles.length);

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
  const uniqueDuplicateFiles: string[] = [];
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
      uniqueDuplicateFiles.push(largestFile);
    }
  }

  console.log(
    "found # unique duplicate html files for crate " + crateName + ":",
    uniqueDuplicateFiles.length
  );

  // Process the unique duplicate files with HTMLReader
  const htmlReader: HTMLReader = new HTMLReader();
  const docs: Document<Metadata>[][] = await Promise.all(
    uniqueDuplicateFiles.map(async (filePath: string) => {
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
  console.log("found # docs from unique duplicates:", foundDocs.length);
  return foundDocs;
}

export function combineDocuments(
  docs1: Document<Metadata>[],
  docs2: Document<Metadata>[]
): Document<Metadata>[] {
  return [...docs1, ...docs2];
}