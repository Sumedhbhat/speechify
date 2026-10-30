export const pages = ['Dictation', 'History', 'Models', 'Learning', 'Settings'] as const;

export function escapeHtml(value: unknown): string {
  return String(value ?? '').replace(
    /[&<>"']/g,
    (character) =>
      ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[character]!,
  );
}

export function button(action: string, text: string, attributes = ''): string {
  return /* HTML */ `<button data-action="${action}" ${attributes}>${text}</button>`;
}

export function notify(message: unknown): void {
  const element = document.querySelector('#message');
  if (element) {
    element.textContent = String(message);
    element.classList.add('visible');
  }
}
