import sys

print(sys.argv[1])
f = open(sys.argv[1], 'r')
lines = f.readlines()

count=0
tsum=0
for line in lines:
    client_ms = int(line.split(" ")[1].split(',')[1])*1000+500
    server_ms = int(line.split(" ")[12].split('.')[1])//1000
    count += 1
    tsum += (client_ms - server_ms)
    print(client_ms, server_ms)
print("Count: ", count, "\nMean Error: ", tsum/count, "micros")