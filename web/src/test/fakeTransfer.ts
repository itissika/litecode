/** jsdom does not implement DataTransfer. Tests build the slice they need. */
export function fakeTransfer(options: {
  data?: Record<string, string>;
  files?: File[];
} = {}): DataTransfer {
  const data = new Map<string, string>(Object.entries(options.data ?? {}));
  const files = options.files ?? [];
  return {
    dropEffect: "none",
    effectAllowed: "all",
    files: Object.assign([...files], {
      item: (index: number) => files[index] ?? null,
    }) as unknown as FileList,
    items: {
      add: (file: File) => {
        files.push(file);
      },
      length: files.length,
    } as unknown as DataTransferItemList,
    get types() {
      return [...data.keys()];
    },
    getData: (type: string) => data.get(type) ?? "",
    setData: (type: string, value: string) => {
      data.set(type, value);
    },
    clearData: (type?: string) => {
      if (type) data.delete(type);
      else data.clear();
    },
    setDragImage: () => {},
  };
}
