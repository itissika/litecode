import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";

import { ConfirmDialog, type ConfirmDialogProps } from "./ConfirmDialog";

export type ConfirmOptions = Omit<ConfirmDialogProps, "onConfirm" | "onCancel">;

interface ConfirmRequest extends ConfirmOptions {
  resolve: (value: boolean) => void;
}

/**
 * Per-panel confirmation queue. Rendering the returned `dialog` inside the
 * calling panel keeps it in that panel's document (including popouts) without
 * a global host or provider.
 */
export function useConfirmDialog(): {
  confirm: (options: ConfirmOptions) => Promise<boolean>;
  dialog: ReactNode;
} {
  const [request, setRequest] = useState<ConfirmRequest | null>(null);
  const requestRef = useRef<ConfirmRequest | null>(null);
  requestRef.current = request;

  const confirm = useCallback((options: ConfirmOptions) => {
    return new Promise<boolean>((resolve) => {
      setRequest((current) => {
        current?.resolve(false);
        return { ...options, resolve };
      });
    });
  }, []);

  useEffect(() => {
    return () => {
      requestRef.current?.resolve(false);
    };
  }, []);

  const settle = (value: boolean) => {
    setRequest((current) => {
      current?.resolve(value);
      return null;
    });
  };

  return {
    confirm,
    dialog: request ? (
      <ConfirmDialog
        {...request}
        onConfirm={() => settle(true)}
        onCancel={() => settle(false)}
      />
    ) : null,
  };
}
