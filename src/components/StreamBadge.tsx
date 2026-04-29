interface Props {
  count: number;
}

export default function StreamBadge({ count }: Props) {
  if (count <= 0) return null;

  return (
    <span className="inline-flex items-center gap-1.5 px-3 py-1 bg-green-900/40 border border-green-700/50 rounded-full text-green-400 text-xs font-semibold animate-pulse">
      <span className="w-2 h-2 rounded-full bg-green-500" />
      {count} stream{count > 1 ? "s" : ""} detectado{count > 1 ? "s" : ""}
    </span>
  );
}
