const NESTED_CONTROL = "a, button, input, textarea, select, summary, [tabindex]";

/**
 * 行全体を押せるカードで、行の中のリンク・ボタン・フォーカスできる要素を押したか。
 * 探すのは行（`currentTarget`）の内側だけにする。`AppShell` の `<main tabIndex={-1}>` のような
 * 行の外の祖先に当たって、行のクリックを常に無視しないようにするため。
 */
export function isNestedControlClick(event: {
  target: EventTarget | null;
  currentTarget: EventTarget | null;
}): boolean {
  const { target, currentTarget } = event;
  if (!(target instanceof Element) || !(currentTarget instanceof Element)) return true;
  const control = target.closest(NESTED_CONTROL);
  return control !== null && control !== currentTarget && currentTarget.contains(control);
}
