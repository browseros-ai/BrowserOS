package proc

import (
	"bufio"
	"fmt"

	"github.com/fatih/color"
)

type Tag struct {
	Name  string
	Color *color.Color
}

var (
	TagBuild   = Tag{"build", color.New(color.FgYellow)}
	TagSetup   = Tag{"setup", color.New(color.FgHiYellow)}
	TagAgent   = Tag{"agent", color.New(color.FgMagenta)}
	TagServer  = Tag{"server", color.New(color.FgCyan)}
	TagBrowser = Tag{"browser", color.New(color.FgBlue)}
	TagWeb     = Tag{"web", color.New(color.FgHiMagenta)}
	TagInfo    = Tag{"info", color.New(color.FgGreen)}
	TagTest    = Tag{"test", color.New(color.FgWhite)}

	ErrorColor = color.New(color.FgRed)
	WarnColor  = color.New(color.FgYellow)
	BoldColor  = color.New(color.Bold)
	DimColor   = color.New(color.Faint)
)

func LogMsg(t Tag, msg string) {
	fmt.Printf("%s %s\n", t.Color.Sprintf("[%s]", t.Name), msg)
}

func LogMsgf(t Tag, format string, args ...any) {
	LogMsg(t, fmt.Sprintf(format, args...))
}

func StreamLines(r interface{ Read([]byte) (int, error) }, t Tag) {
	scanner := bufio.NewScanner(r)
	scanner.Buffer(make([]byte, 0, 64*1024), 1024*1024)
	for scanner.Scan() {
		line := scanner.Text()
		if line != "" {
			fmt.Printf("%s %s\n", t.Color.Sprintf("[%s]", t.Name), line)
		}
	}
}
