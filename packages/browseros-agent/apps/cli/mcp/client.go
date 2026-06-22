package mcp

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"io"
	"math"
	"net/http"
	"time"

	sdkmcp "github.com/modelcontextprotocol/go-sdk/mcp"
)

type Client struct {
	BaseURL    string
	HTTPClient *http.Client
	Version    string
	Debug      bool
}

func NewClient(baseURL, version string, timeout time.Duration) *Client {
	return &Client{
		BaseURL: baseURL,
		HTTPClient: &http.Client{
			Timeout: timeout,
		},
		Version: version,
	}
}

func (c *Client) connect(ctx context.Context) (*sdkmcp.ClientSession, error) {
	sdkClient := sdkmcp.NewClient(&sdkmcp.Implementation{
		Name:    "browseros-cli",
		Version: c.Version,
	}, nil)

	transport := &sdkmcp.StreamableClientTransport{
		Endpoint:             c.BaseURL + "/mcp",
		HTTPClient:           c.HTTPClient,
		DisableStandaloneSSE: true,
	}

	session, err := sdkClient.Connect(ctx, transport, nil)
	if err != nil {
		return nil, fmt.Errorf("cannot connect to BrowserOS at %s: %w%s", c.BaseURL, err, connectionSetupInstructions())
	}
	return session, nil
}

// CallTool connects, initializes, calls the named tool, and returns the result.
func (c *Client) CallTool(name string, args map[string]any) (*ToolResult, error) {
	ctx, cancel := context.WithTimeout(context.Background(), c.HTTPClient.Timeout)
	defer cancel()

	session, err := c.connect(ctx)
	if err != nil {
		return nil, err
	}
	defer session.Close()

	if args == nil {
		args = map[string]any{}
	}

	sdkResult, err := session.CallTool(ctx, &sdkmcp.CallToolParams{
		Name:      name,
		Arguments: args,
	})
	if err != nil {
		return nil, err
	}

	result := convertResult(sdkResult)
	if result.IsError {
		return result, fmt.Errorf("%s", result.TextContent())
	}

	return result, nil
}

func convertResult(r *sdkmcp.CallToolResult) *ToolResult {
	result := &ToolResult{
		IsError: r.IsError,
	}

	for _, c := range r.Content {
		switch v := c.(type) {
		case *sdkmcp.TextContent:
			result.Content = append(result.Content, ContentItem{Type: "text", Text: v.Text})
		case *sdkmcp.ImageContent:
			result.Content = append(result.Content, ContentItem{Type: "image", Data: base64.StdEncoding.EncodeToString(v.Data), MimeType: v.MIMEType})
		}
	}

	if r.StructuredContent != nil {
		switch sc := r.StructuredContent.(type) {
		case map[string]any:
			result.StructuredContent = sc
		default:
			data, err := json.Marshal(sc)
			if err == nil {
				var m map[string]any
				if json.Unmarshal(data, &m) == nil {
					result.StructuredContent = m
				}
			}
		}
	}

	return result
}

// ResolvePageID returns the explicit page ID or fetches the active page.
func (c *Client) ResolvePageID(explicit *int) (int, error) {
	if explicit != nil {
		return *explicit, nil
	}
	result, err := c.CallTool("tabs", map[string]any{"action": "list"})
	if err != nil {
		return 0, fmt.Errorf("no pages available: %w", err)
	}

	if pageID, ok := extractPageID(result.StructuredContent); ok {
		return pageID, nil
	}

	return 0, fmt.Errorf("could not determine active page ID from response")
}

func extractPageID(sc map[string]any) (int, bool) {
	if sc == nil {
		return 0, false
	}
	if pageID, ok := intValue(sc["pageId"]); ok {
		return pageID, true
	}
	if pageID, ok := intValue(sc["page"]); ok {
		return pageID, true
	}
	page, ok := sc["page"].(map[string]any)
	if !ok {
		pages, ok := sc["pages"].([]any)
		if !ok || len(pages) == 0 {
			return 0, false
		}
		for _, item := range pages {
			pageInfo, ok := item.(map[string]any)
			if !ok {
				continue
			}
			if isActive, _ := pageInfo["isActive"].(bool); isActive {
				if pageID, ok := extractPageID(pageInfo); ok {
					return pageID, true
				}
			}
		}
		if first, ok := pages[0].(map[string]any); ok {
			return extractPageID(first)
		}
		return 0, false
	}
	pageID, ok := intValue(page["pageId"])
	if ok {
		return pageID, true
	}
	return intValue(page["page"])
}

func intValue(v any) (int, bool) {
	switch n := v.(type) {
	case int:
		return n, true
	case int32:
		return int(n), true
	case int64:
		return int(n), true
	case float64:
		if math.Trunc(n) != n {
			return 0, false
		}
		return int(n), true
	case json.Number:
		i, err := n.Int64()
		if err != nil {
			return 0, false
		}
		return int(i), true
	default:
		return 0, false
	}
}

// Health checks the /health endpoint (REST, not MCP).
func (c *Client) Health() (map[string]any, error) {
	return c.restGET("/health")
}

// Status checks the /status endpoint (REST, not MCP).
func (c *Client) Status() (map[string]any, error) {
	return c.restGET("/status")
}

func (c *Client) restGET(path string) (map[string]any, error) {
	resp, err := c.HTTPClient.Get(c.BaseURL + path)
	if err != nil {
		return nil, fmt.Errorf("cannot connect to BrowserOS at %s: %w%s", c.BaseURL, err, connectionSetupInstructions())
	}
	defer resp.Body.Close()

	if resp.StatusCode >= 400 {
		body, _ := io.ReadAll(resp.Body)
		return nil, fmt.Errorf("server returned HTTP %d: %s", resp.StatusCode, string(body))
	}

	var data map[string]any
	if err := json.NewDecoder(resp.Body).Decode(&data); err != nil {
		return nil, fmt.Errorf("parse response: %w", err)
	}
	return data, nil
}

// connectionSetupInstructions explains how to recover from a stale or missing server URL.
func connectionSetupInstructions() string {
	return "\n\n" +
		"  Open BrowserOS Settings > BrowserOS MCP and copy the Server URL.\n" +
		"  Save it with:       browseros-cli init <Server URL>\n" +
		"  Example:            browseros-cli init http://127.0.0.1:9000/mcp\n" +
		"  Run once with:      browseros-cli --server <Server URL> health\n" +
		"  If BrowserOS is closed:  browseros-cli launch\n" +
		"  If not installed:        download from https://browseros.com"
}
