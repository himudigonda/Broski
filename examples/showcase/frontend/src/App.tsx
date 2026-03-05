import { useEffect, useMemo, useState } from 'react';

type Health = {
  status: string;
  service: string;
  timestamp_utc: string;
};

type Metric = {
  name: string;
  value: string;
  detail: string;
};

export function App() {
  const [health, setHealth] = useState<Health | null>(null);
  const [metrics, setMetrics] = useState<Metric[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const fetchData = async () => {
      try {
        const [healthRes, metricsRes] = await Promise.all([
          fetch('/api/health'),
          fetch('/api/metrics'),
        ]);

        if (!healthRes.ok || !metricsRes.ok) {
          throw new Error('backend request failed');
        }

        const healthData = (await healthRes.json()) as Health;
        const metricsData = (await metricsRes.json()) as { metrics: Metric[] };
        setHealth(healthData);
        setMetrics(metricsData.metrics);
      } catch (err) {
        setError((err as Error).message);
      }
    };

    fetchData();
  }, []);

  const summary = useMemo(() => {
    if (!health) {
      return 'Fetching system status...';
    }
    return `${health.service} is ${health.status}`;
  }, [health]);

  return (
    <main className="page">
      <header className="hero">
        <p className="eyebrow">Please v0.5.0 Showcase</p>
        <h1>Build Graph Telemetry Dashboard</h1>
        <p>{summary}</p>
      </header>

      {error ? <p className="error">Error loading API data: {error}</p> : null}

      <section className="grid">
        <article className="card">
          <h2>Health</h2>
          <dl>
            <div>
              <dt>Status</dt>
              <dd>{health?.status ?? 'loading'}</dd>
            </div>
            <div>
              <dt>Service</dt>
              <dd>{health?.service ?? 'loading'}</dd>
            </div>
            <div>
              <dt>UTC Time</dt>
              <dd>{health?.timestamp_utc ?? 'loading'}</dd>
            </div>
          </dl>
        </article>

        <article className="card card-wide">
          <h2>Execution Signals</h2>
          <table>
            <thead>
              <tr>
                <th>Metric</th>
                <th>Value</th>
                <th>Detail</th>
              </tr>
            </thead>
            <tbody>
              {metrics.map((metric) => (
                <tr key={metric.name}>
                  <td>{metric.name}</td>
                  <td>{metric.value}</td>
                  <td>{metric.detail}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </article>
      </section>
    </main>
  );
}
