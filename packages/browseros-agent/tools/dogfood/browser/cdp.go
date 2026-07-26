package browser

import (
	"context"
	"fmt"
	"net/http"
	"time"
)

func WaitForCDP(ctx context.Context, port int, maxAttempts int) bool {
	client := &http.Client{Timeout: time.Second}
	url := fmt.Sprintf("http://127.0.0.1:%d/json/version", port)

	for range maxAttempts {
		if ctx.Err() != nil {
			return false
		}
		resp, err := client.Get(url)
		if err == nil {
			resp.Body.Close()
			if resp.StatusCode == 200 {
				return true
			}
		}
		select {
		case <-ctx.Done():
			return false
		case <-time.After(500 * time.Millisecond):
		}
	}
	return false
}
