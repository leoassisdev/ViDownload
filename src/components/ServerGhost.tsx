export default function ServerGhost() {
  return (
    <div className="container_SevMini">
      <svg
        className="SevMini"
        width="120"
        height="120"
        viewBox="0 0 200 200"
        fill="none"
        xmlns="http://www.w3.org/2000/svg"
      >
        {/* Server body */}
        <rect x="40" y="30" width="120" height="140" rx="12" fill="#1a1a2e" stroke="#0f0" strokeWidth="2" />

        {/* Screen */}
        <rect x="55" y="45" width="90" height="50" rx="4" fill="#0a0a1a" stroke="#0f0" strokeWidth="1" />

        {/* Screen content lines */}
        <line x1="62" y1="58" x2="110" y2="58" stroke="#0f0" strokeWidth="2" opacity="0.7" />
        <line x1="62" y1="68" x2="95" y2="68" stroke="#0f0" strokeWidth="2" opacity="0.5" />
        <line x1="62" y1="78" x2="125" y2="78" stroke="#0f0" strokeWidth="2" opacity="0.3" />

        {/* LED indicators */}
        <circle id="strobe_led1" cx="65" cy="115" r="4" fill="#0f0" />
        <circle id="strobe_color1" cx="80" cy="115" r="4" fill="#17e300" />
        <circle id="strobe_color3" cx="95" cy="115" r="4" fill="#ff5f4a" />

        {/* Drive slots */}
        <rect x="60" y="130" width="80" height="8" rx="2" fill="#0a0a1a" stroke="#333" strokeWidth="1" />
        <rect x="60" y="145" width="80" height="8" rx="2" fill="#0a0a1a" stroke="#333" strokeWidth="1" />
      </svg>

      <svg
        className="Ghost"
        width="80"
        height="30"
        viewBox="0 0 200 50"
        fill="none"
        xmlns="http://www.w3.org/2000/svg"
      >
        <ellipse cx="100" cy="25" rx="60" ry="12" fill="#0f03" />
      </svg>
    </div>
  );
}
