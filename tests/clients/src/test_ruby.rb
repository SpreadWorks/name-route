require 'pg'
require 'mysql2'

pg_port = ENV['PG_PORT']
my_port = ENV['MY_PORT']

begin
  conn = PG.connect(host: '127.0.0.1', port: pg_port,
                    user: 'user', password: 'pass', dbname: 'app',
                    sslmode: 'disable')
  res = conn.exec('SELECT 1 AS v')
  raise 'unexpected' unless res[0]['v'] == '1'
  conn.close
  puts 'PG:PASS'
rescue => e
  puts "PG:FAIL:#{e.message}"
end

begin
  client = Mysql2::Client.new(host: '127.0.0.1', port: my_port.to_i,
                              username: 'root', password: '', database: 'myapp')
  results = client.query('SELECT 1 AS v')
  raise 'unexpected' unless results.first['v'] == 1
  client.close
  puts 'MySQL:PASS'
rescue => e
  puts "MySQL:FAIL:#{e.message}"
end
