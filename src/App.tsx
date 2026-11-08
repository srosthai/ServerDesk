import "./App.css";

function App() {
  return (
    <main className="launch">
      <header className="launch__brand">
        <span className="launch__mark" aria-hidden="true">
          S
        </span>
        <span>ServerDesk</span>
      </header>

      <section className="launch__body">
        <p className="launch__status">
          <span className="launch__dot" aria-hidden="true" />
          Desktop shell ready
        </p>
        <h1>Every server, one desk.</h1>
        <p className="launch__lede">
          Connect over SSH, inspect system health, manage services, and work with files without
          leaving the app.
        </p>
      </section>

      <footer className="launch__footer">
        <span>Local-first</span>
        <span>v0.1.0</span>
      </footer>
    </main>
  );
}

export default App;
