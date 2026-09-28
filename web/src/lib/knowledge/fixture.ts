import folders from "./knowledge.folders.json";
import raw from "./knowledge.fixture.json";

import type { KnowledgeFolder, KnowledgeNode } from "./types";

export const knowledgeFixture = raw as KnowledgeNode[];
export const knowledgeFolderFixture = folders as KnowledgeFolder[];
