import type { ReactNode } from "react";

const INLINE = /(\*\*[^*]+\*\*|`[^`]+`|\[[^\]]+\]\([^)]+\)|https?:\/\/\S+)/g;

function inline(text: string): ReactNode[] {
  return text.split(INLINE).map((part, i) => {
    if (part.startsWith("**")) return <strong key={i}>{part.slice(2, -2)}</strong>;
    if (part.startsWith("`")) return <code key={i}>{part.slice(1, -1)}</code>;
    const link = part.match(/^\[([^\]]+)\]\(/);
    if (link) return link[1];
    return part;
  });
}

/** Small renderer for GitHub release notes: headings, lists, code blocks, bold, code. Links show as text. */
export function Markdown({ source }: { source: string }) {
  const out: ReactNode[] = [];
  const lines = source.replace(/\r\n?/g, "\n").split("\n");
  let list: string[] = [];
  let para: string[] = [];
  const flush = () => {
    if (para.length) out.push(<p key={out.length}>{inline(para.join(" "))}</p>);
    if (list.length)
      out.push(
        <ul key={out.length}>
          {list.map((l, i) => (
            <li key={i}>{inline(l)}</li>
          ))}
        </ul>,
      );
    para = [];
    list = [];
  };
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    if (line.startsWith("```")) {
      flush();
      const code: string[] = [];
      while (++i < lines.length && !lines[i].startsWith("```")) code.push(lines[i]);
      out.push(<pre key={out.length}>{code.join("\n")}</pre>);
      continue;
    }
    const h = line.match(/^#{1,6}\s+(.*)/);
    const li = line.match(/^\s*[-*]\s+(.*)/);
    if (h) {
      flush();
      out.push(<h4 key={out.length}>{inline(h[1])}</h4>);
    } else if (li) {
      if (para.length) flush();
      list.push(li[1]);
    } else if (!line.trim()) {
      flush();
    } else {
      if (list.length) flush();
      para.push(line.trim());
    }
  }
  flush();
  return <div className="md">{out}</div>;
}
