package release

import "testing"

func TestValidateOptions(t *testing.T) {
	cases := []struct {
		name    string
		opts    Options
		wantErr bool
	}{
		{name: "show modules only", opts: Options{ShowModules: true}, wantErr: false},
		{name: "no flags", opts: Options{}, wantErr: true},
		{name: "list without version", opts: Options{List: true}, wantErr: false},
		{name: "appcast requires version", opts: Options{Appcast: true}, wantErr: true},
		{name: "publish with version", opts: Options{Publish: true, Version: "0.31.0"}, wantErr: false},
		{name: "download invalid os", opts: Options{Download: true, Version: "0.31.0", OSFilter: "bsd"}, wantErr: true},
	}

	for _, tc := range cases {
		err := ValidateOptions(tc.opts)
		if tc.wantErr && err == nil {
			t.Fatalf("%s: expected error", tc.name)
		}
		if !tc.wantErr && err != nil {
			t.Fatalf("%s: unexpected error: %v", tc.name, err)
		}
	}
}
