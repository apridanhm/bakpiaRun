<?php 

$fibers = [];

for ($i = 0; $i < 1000; $i++) {

    $fibers[] = new Fiber(function ($id) {

        $sum = 0;

        for ($j = 0; $j < 1000; $j++) {
            $sum += $j;
        }

        Fiber::suspend($sum);

        return $id;
    });
}

foreach ($fibers as $idx => $fiber) {
    $fiber->start($idx);
}

foreach ($fibers as $fiber) {
    $fiber->resume();
}

echo "Fiber stress test OK\n";