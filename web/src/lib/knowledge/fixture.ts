import { knowledgeFromFiles } from "./document";
import { knowledgeCorpusFiles } from "./corpus";

const indexed = knowledgeFromFiles(knowledgeCorpusFiles);

export const knowledgeFixture = indexed.nodes;
export const knowledgeFolderFixture = indexed.folders;
