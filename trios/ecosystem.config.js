module.exports = {
  apps: [
    {
      name: 'clade-monitor',
      script: './target/release/clade-monitor',
      cwd: '/Users/playra/BrowserOS-full/trios',
      env: {
        TRIOS_ROOT: '/Users/playra/BrowserOS-full/trios',
        TRIOS_PORT_SOVEREIGN: '9105',
        TRIOS_PORT_A2A: '9200',
        TRIOS_PORT_CANARY: '9205',
      },
      autorestart: true,
      max_restarts: 10,
      restart_delay: 5000,
    },
    {
      name: 'clade-dashboard',
      script: './target/release/clade-dashboard',
      cwd: '/Users/playra/BrowserOS-full/trios',
      env: {
        TRIOS_ROOT: '/Users/playra/BrowserOS-full/trios',
        TRIOS_PORT_DASHBOARD: '9405',
        TRIOS_PORT_SOVEREIGN: '9105',
        TRIOS_PORT_CANARY: '9205',
      },
      autorestart: true,
      max_restarts: 10,
      restart_delay: 3000,
    },
  ],
};
