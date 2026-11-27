import { useCallback, useState } from "react";

/**
 * Home のタイムライン/post 画面で 1 件だけ編集するための状態。
 * 保存リクエストは呼び出し元が行い、このフックは入力・進行中・エラーだけを持つ。
 */
export function usePostEditState() {
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editText, setEditText] = useState("");
  const [editError, setEditError] = useState(false);
  const [editPending, setEditPending] = useState(false);

  const startEdit = useCallback((id: string, content: string) => {
    setEditingId(id);
    setEditText(content);
    setEditError(false);
  }, []);

  const cancelEdit = useCallback(() => {
    setEditingId(null);
  }, []);

  return {
    editingId,
    editText,
    setEditText,
    editError,
    setEditError,
    editPending,
    setEditPending,
    startEdit,
    cancelEdit,
  };
}
