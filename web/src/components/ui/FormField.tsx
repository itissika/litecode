import type {
  InputHTMLAttributes,
  ReactNode,
  TextareaHTMLAttributes,
} from "react";

export function FieldLabel({
  children,
  required,
  htmlFor,
}: {
  children: ReactNode;
  required?: boolean;
  htmlFor?: string;
}) {
  return (
    <label
      htmlFor={htmlFor}
      className="mb-0.5 block text-xs font-medium text-(--_dk-text-muted)"
    >
      {children}
      {required ? <span className="text-(--_dk-red-500)"> *</span> : null}
    </label>
  );
}

export function TextInput(props: InputHTMLAttributes<HTMLInputElement>) {
  return (
    <input {...props} className={`settings-input ${props.className ?? ""}`} />
  );
}

export function TextArea(props: TextareaHTMLAttributes<HTMLTextAreaElement>) {
  return (
    <textarea
      {...props}
      className={`settings-input ${props.className ?? ""}`}
    />
  );
}
