/** Agent bash live-log RPCs (tee tail + kill). Not the human PTY API. */

import { useConnectionStore } from "../stores/connectionStore";
import type { BashTailResult } from "../api/types";

export async function bashTail(bashId: string): Promise<BashTailResult> {
  return useConnectionStore.getState().sendRpc<BashTailResult>("bash/tail", {
    bash_id: bashId,
  });
}

export async function bashKill(bashId: string): Promise<void> {
  await useConnectionStore.getState().sendRpc("bash/kill", { bash_id: bashId });
}
