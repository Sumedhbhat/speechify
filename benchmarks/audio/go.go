package main

import (
	"encoding/binary"
	"encoding/json"
	"math"
	"os"
	"strconv"
	"time"
)

var sink []byte

func run(a []float32, silence bool) []byte {
	if silence {
		sum := float64(0)
		for _, v := range a {
			sum += float64(v * v)
		}
		quiet := len(a) == 0 || float32(math.Sqrt(float64(float32(sum/float64(len(a)))))) < 0.004
		if quiet {
			return []byte{1}
		}
		return []byte{0}
	}
	pcm := make([]byte, len(a)*2)
	for i, v := range a {
		if v < -1 {
			v = -1
		}
		if v > 1 {
			v = 1
		}
		binary.LittleEndian.PutUint16(pcm[i*2:], uint16(int16(v*32767)))
	}
	return pcm
}
func main() {
	b, e := os.ReadFile(os.Args[1])
	if e != nil {
		panic(e)
	}
	a := make([]float32, len(b)/4)
	for i := range a {
		a[i] = math.Float32frombits(binary.LittleEndian.Uint32(b[i*4:]))
	}
	count, e := strconv.Atoi(os.Args[3])
	if e != nil {
		panic(e)
	}
	silence := os.Args[2] == "silence"
	for i := 0; i < 30; i++ {
		sink = run(a, silence)
	}
	start := time.Now()
	for i := 0; i < count; i++ {
		sink = run(a, silence)
	}
	ns := float64(time.Since(start).Nanoseconds()) / float64(count)
	if e = os.WriteFile(os.Args[4], sink, 0600); e != nil {
		panic(e)
	}
	json.NewEncoder(os.Stdout).Encode(map[string]float64{"ns_per_op": ns})
}
