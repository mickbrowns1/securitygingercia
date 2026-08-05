export default function HelpOverlay({ onClose }) {
  return (
    <div
      id="help-overlay"
      className="help-overlay"
      onClick={(ev) => {
        if (ev.target.id === "help-overlay") onClose();
      }}
    >
      <div className="help-panel">
        <h2>Keyboard shortcuts</h2>
        <table>
          <tbody>
            <tr>
              <td>
                <kbd>h</kbd> <kbd>l</kbd> <kbd>t</kbd>
              </td>
              <td>Jump to Health / Logs / Topology</td>
            </tr>
            <tr>
              <td>
                <kbd>/</kbd>
              </td>
              <td>Focus the log search box</td>
            </tr>
            <tr>
              <td>
                <kbd>n</kbd> <kbd>p</kbd>
              </td>
              <td>Next / previous ERROR entry (Logs tab)</td>
            </tr>
            <tr>
              <td>
                <kbd>?</kbd>
              </td>
              <td>Show / hide this help</td>
            </tr>
            <tr>
              <td>
                <kbd>Esc</kbd>
              </td>
              <td>Clear search/filter, or close this help</td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>
  );
}
