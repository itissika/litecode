import { useEffect } from "react";

import i18n from "../i18n";
import { useConfirmDialog } from "./ui/useConfirmDialog";

/**
 * The close question, in the app's own dialog. The host asks when a session is
 * running and waits for the answer; mounting this once at the app root keeps the
 * dialog in the window that is being closed, styled like everything else.
 */
export function WindowCloseGuard() {
  const { confirm, dialog } = useConfirmDialog();

  useEffect(() => {
    const unsubscribe = window.litecode?.onCloseRequest?.(() => {
      // Acknowledge before the human starts thinking: `shown` stops the host's
      // fallback timer, so the native prompt never stacks on this one.
      void window.litecode?.answerCloseRequest?.("shown");
      void confirm({
        title: i18n.t("chrome.closeRunning.title"),
        body: i18n.t("chrome.closeRunning.body"),
        confirmLabel: i18n.t("chrome.closeRunning.confirm"),
        cancelLabel: i18n.t("chrome.closeRunning.cancel"),
        danger: true,
      }).then((close) =>
        window.litecode?.answerCloseRequest?.(close ? "close" : "cancel"),
      );
    });
    return () => unsubscribe?.();
  }, [confirm]);

  return <>{dialog}</>;
}
