/** Pfeiltasten in Listen: Fokus und Auswahl auf die Nachbarzeile verschieben. */
export function onRowKeydown(e: KeyboardEvent) {
  if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
  e.preventDefault();
  const li = (e.currentTarget as HTMLElement).closest("li");
  const sibling = e.key === "ArrowDown" ? li?.nextElementSibling : li?.previousElementSibling;
  const button = sibling?.querySelector("button");
  button?.focus();
  button?.click();
}
