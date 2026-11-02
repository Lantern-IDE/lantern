// 지도 노드의 열쇠 (순수 함수: 단위 테스트 대상)

/** 다시 그려도 같은 노드로 치는 열쇠. 심볼 id는 파일을 다시 인덱싱하면 바뀐다.
 *  한 파일에 같은 이름이 여럿이면(Go의 String 메서드) 순번으로 가른다: 아니면 저장 뒤 한 자리에 포개진다 */
export function nodeKeys(list: { kind: string; path?: string; label: string }[]): string[] {
  const seen = new Map<string, number>();
  return list.map((n) => {
    const k = `${n.kind}:${n.path ?? ""}:${n.label}`;
    const i = seen.get(k) ?? 0;
    seen.set(k, i + 1);
    return `${k}#${i}`;
  });
}
