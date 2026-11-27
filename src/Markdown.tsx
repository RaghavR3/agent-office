import { Fragment, ReactNode } from "react";
import { api } from "./api";

// A small renderer that builds React nodes directly, so agent output never reaches innerHTML.

// Web links, plus plain-http previews running on this Mac (localhost, 127.0.0.1, 0.0.0.0).
const WEB = String.raw`(?:https:\/\/|http:\/\/(?:localhost|127\.0\.0\.1|0\.0\.0\.0)\b)`;
const INLINE = new RegExp(String.raw`(\x60[^\x60]+\x60|\*\*[^*]+\*\*|\[[^\]]+\]\(${WEB}[^)\s]*\)|${WEB}[^\s)<>"'\x60]*)`, "g");
const MD_LINK = new RegExp(String.raw`^\[([^\]]+)\]\((${WEB}[^)\s]*)\)$`);
const IS_URL = new RegExp(`^${WEB}\\S*$`);

function Link({ url, children }: { url: string; children: ReactNode }) {
  return (
    <a
      href={url}
      onClick={(e) => {
        e.preventDefault();
        api.openLink(url);
      }}
    >
      {children}
    </a>
  );
}

function inline(text: string): ReactNode[] {
  return text.split(INLINE).map((part, i) => {
    if (!part) return null;
    if (part.startsWith("`") && part.endsWith("`")) {
      const code = part.slice(1, -1);
      // A code span that's only an address, like `http://localhost:5173`, still opens.
      return IS_URL.test(code) ? (
        <Link key={i} url={code}>
          <code>{code}</code>
        </Link>
      ) : (
        <code key={i}>{code}</code>
      );
    }
    if (part.startsWith("**")) return <strong key={i}>{part.slice(2, -2)}</strong>;
    const md = part.match(MD_LINK);
    if (md) return <Link key={i} url={md[2]}>{md[1]}</Link>;
    if (IS_URL.test(part)) {
      const url = part.replace(/[.,;:]+$/, "");
      return (
        <Fragment key={i}>
          <Link url={url}>{url.replace(/^https?:\/\//, "")}</Link>
          {part.slice(url.length)}
        </Fragment>
      );
    }
    return <Fragment key={i}>{part}</Fragment>;
  });
}

export function Markdown({ text }: { text: string }) {
  const lines = text.replace(/\r/g, "").split("\n");
  const out: ReactNode[] = [];
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    if (line.startsWith("```")) {
      const body: string[] = [];
      i++;
      while (i < lines.length && !lines[i].startsWith("```")) body.push(lines[i++]);
      i++;
      out.push(<pre key={out.length}>{body.join("\n")}</pre>);
      continue;
    }
    if (/^\s*\|/.test(line)) {
      const rows: string[][] = [];
      while (i < lines.length && /^\s*\|/.test(lines[i])) {
        const cells = lines[i].trim().replace(/^\||\|$/g, "").split("|").map((c) => c.trim());
        if (!cells.every((c) => /^:?-+:?$/.test(c))) rows.push(cells);
        i++;
      }
      out.push(
        <table key={out.length}>
          <tbody>
            {rows.map((r, ri) => (
              <tr key={ri}>{r.map((c, ci) => (ri === 0 ? <th key={ci}>{inline(c)}</th> : <td key={ci}>{inline(c)}</td>))}</tr>
            ))}
          </tbody>
        </table>,
      );
      continue;
    }
    if (/^\s*([-*]|\d+\.)\s+/.test(line)) {
      const ordered = /^\s*\d+\./.test(line);
      const items: string[] = [];
      while (i < lines.length && /^\s*([-*]|\d+\.)\s+/.test(lines[i])) {
        items.push(lines[i].replace(/^\s*([-*]|\d+\.)\s+/, ""));
        i++;
      }
      const List = ordered ? "ol" : "ul";
      out.push(<List key={out.length}>{items.map((it, k) => <li key={k}>{inline(it)}</li>)}</List>);
      continue;
    }
    if (!line.trim()) {
      i++;
      continue;
    }
    const para: string[] = [];
    while (i < lines.length && lines[i].trim() && !/^(```|\s*\||\s*([-*]|\d+\.)\s+)/.test(lines[i])) {
      para.push(lines[i].replace(/^#+\s*/, ""));
      i++;
    }
    const heading = /^#+\s/.test(line);
    out.push(heading ? <h4 key={out.length}>{inline(para.join(" "))}</h4> : <p key={out.length}>{inline(para.join(" "))}</p>);
  }
  return <div className="md">{out}</div>;
}
