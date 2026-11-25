export function splitList(value: string | undefined): string[] {
  return (value ?? '')
    .split(/[,，]/)
    .map((item) => item.trim())
    .filter(Boolean)
}

export function joinList(values: string[] | undefined): string {
  return (values ?? []).join('，')
}
