const path = require('path')
const HtmlWebpackPlugin = require('html-webpack-plugin')
const CopyPlugin = require('copy-webpack-plugin')
const webpack = require('webpack')
const TerserPlugin = require('terser-webpack-plugin')
const dotenv = require('dotenv')
const fs = require('fs')
const ExtensionReloader = require('webpack-ext-reloader')

// Load .env file
let envKeys = {}

// Load environment variables from .env file
const env = dotenv.config()
envKeys = env.parsed || {}

// Validate required API keys
if (!env.parsed) {
  throw new Error('No .env file found. API keys must be set manually.')
}

// Create environment variables to inject
const processEnv = {
  'process.env.POSTHOG_API_KEY': JSON.stringify(envKeys.POSTHOG_API_KEY || ''),
  'process.env.KLAVIS_API_KEY': JSON.stringify(envKeys.KLAVIS_API_KEY || ''),
  'process.env.NODE_ENV': JSON.stringify(process.env.NODE_ENV || 'development'),
  'process.env.MOONDREAM_API_KEY': JSON.stringify(envKeys.MOONDREAM_API_KEY || ''),
  'process.env.OPENAI_API_KEY': JSON.stringify(envKeys.OPENAI_API_KEY || ''),
}

console.log('API keys will be injected at build time (keys hidden for security)')

// Determine environment (default to development if not specified)
const isDevelopment = process.env.NODE_ENV !== 'production'

// Detect if this is a Chrome build
const isChromeTarget = process.env.BUILD_TARGET === 'chrome'

// Detect if webpack is running in watch mode (CLI flag or env variable set by webpack)
const isWatch = process.argv.includes('--watch') || process.env.WEBPACK_WATCH === 'true'

module.exports = {
  mode: isDevelopment ? 'development' : 'production',
  devtool: isDevelopment ? 'source-map' : false,
  entry: {
    sidepanel: './src/sidepanel/index.tsx',
    background: './src/background/index.ts',
    'glow-animation': './src/content/glow-animation.ts',
    newtab: './src/newtab/index.tsx'
  },
  output: {
    path: path.resolve(__dirname, 'dist'),
    filename: '[name].js'
  },
  module: {
    rules: [
      {
        test: /\.tsx?$/,
        use: {
          loader: 'ts-loader',
          options: {
            onlyCompileBundledFiles: true
          }
        },
        exclude: [
          /node_modules/,
          /\.(test|spec)\.(ts|tsx)$/
        ]
      },
      {
        test: /\.scss$/,
        use: [
          'style-loader',
          {
            loader: 'css-loader',
            options: {
              modules: {
                localIdentName: '[name]__[local]--[hash:base64:5]'
              }
            }
          },
          {
            loader: 'sass-loader',
            options: {
              api: 'modern'  // Use the modern Sass API
            }
          }
        ]
      },
      {
        test: /\.css$/,
        use: [
          'style-loader',
          'css-loader',
          'postcss-loader'
        ]
      }
    ]
  },
  resolve: {
    extensions: ['.tsx', '.ts', '.js'],
    alias: {
      '@': path.resolve(__dirname, 'src')
    }
  },
  optimization: {
    splitChunks: false,  // Disable all code splitting
    runtimeChunk: false,  // Keep runtime in each entry point
    // Configure minimizer to prevent LICENSE file generation in production
    minimizer: isDevelopment ? [] : [
      new TerserPlugin({
        extractComments: false,  // Disable LICENSE file extraction
        terserOptions: {
          format: {
            comments: false,  // Remove all comments
          },
          compress: { // Remove console and debugger statements in production
            drop_console: true,
            drop_debugger: true
          },
        },
      }),
    ],
  },
  plugins: [
    // Limit chunks to entry points only - prevents dynamic chunk creation
    // This forces all imports (including dynamic) to be bundled into their parent entry
    new webpack.optimize.LimitChunkCountPlugin({
      maxChunks: 4  // One chunk per entry point (sidepanel, background, glow-animation, newtab)
    }),
    new HtmlWebpackPlugin({
      template: './src/sidepanel/index.html',
      filename: 'sidepanel.html',
      chunks: ['sidepanel']
    }),
    new HtmlWebpackPlugin({
      template: './src/newtab/index.html',
      filename: 'newtab.html',
      chunks: ['newtab']
    }),
    new CopyPlugin({
      patterns: [
        { 
          from: 'manifest.json', 
          to: '.',
          transform: (content) => {
            if (isChromeTarget) {
              const manifest = JSON.parse(content.toString());
              manifest.component = true;
              return JSON.stringify(manifest, null, 2);
            }
            return content;
          }
        },
        { from: 'assets', to: 'assets', noErrorOnMissing: true }
      ]
    }),
    new webpack.DefinePlugin(processEnv),
    // Include hot-reload plugin only when in development AND watch mode to allow interactive dev
    ...(isDevelopment && isWatch
      ? [
          new ExtensionReloader({
            port: 9090,
            reloadPage: true,
            entries: {
              background: 'background',
              contentScript: 'content',
              extensionPage: ['sidepanel']
            }
          })
        ]
      : [])
  ]
}
