export default function TerminalLoader({ text = "Loading..." }: { text?: string }) {
  return (
    <div className="terminal-loader">
      <div className="terminal-header">
        <div className="terminal-title">ViDownload</div>
        <div className="terminal-controls">
          <div className="control close" />
          <div className="control minimize" />
          <div className="control maximize" />
        </div>
      </div>
      <div className="text">{text}</div>
    </div>
  );
}
